//! The CPU-side residency table: which nodes are expanded, which bricks are
//! resident, where their records and blocks live, and the operations that
//! change it. Every operation appends the matching GPU writes to an
//! [`UploadBatch`], so table and pools change together.
//!
//! Invariants, checked by [`Residency::verify`]:
//!
//! - Every node with a record in the pool (the root, and every non-air,
//!   non-uniform child of an expanded node) has an entry whose `record` is
//!   exactly what the pool holds at `index`.
//! - An expanded node's children are contiguous at its group's `first`, in
//!   slot order, and its record is internal with `ref = first`.
//! - A level-1 entry with a brick has a brick record pointing at its block,
//!   whose words equal the encoding of the current source content.
//! - Once every queued change is applied, masks and materials describe the
//!   current source: applying a change rewrites the records on its path in
//!   the same batch, and a changed brick that cannot be uploaded is made not
//!   resident, never left stale. Changes wait in a queue under a per-frame
//!   budget (decision 0012); until one is applied, the records it touches
//!   describe the content before it.
//! - Every expanded entry holds one node group, every resident brick one
//!   brick id, and each entry's child masks match its children's entries.

use bevy::platform::collections::{HashMap, HashSet};
use bevy::tasks::{TaskPool, TaskPoolBuilder};
use world_core::MaterialId;
use world_core::tree::{NodeView, VoxelSource};

use super::batch::UploadBatch;
use super::key::{NodeKey, Shape, view_at};
use super::select::GrowthState;
use super::summaries::{ChunkSummaries, WARMED_LEVEL, summarize_level2_node};
use super::table::Table;
use crate::brick_cache::format::brick::encode_brick;
use crate::brick_cache::format::summary::{
    Content, Summary, Tally, content, count_voxels, exact_summary, node_volume,
};
use crate::brick_cache::format::{BrickRef, GpuNode, Kind, ROOT_NODE};
use crate::brick_cache::pool::alloc::{BrickAlloc, BrickSpace, IdSpace, NodeAlloc, NodeSpace};
use crate::brick_cache::pool::layout::PoolLayout;

/// What a replaced chunk applied with offered summaries is charged against
/// the change budget, besides the summaries it then computes (its own, and
/// those of its ancestors when their paths are rewritten): its offer is
/// inserted in one step, but the chunk still buries what its old content
/// held and rewrites one record per level of its path. A group's worth.
pub const OFFER_CHARGE: u64 = 64;

/// Why an operation did nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The node has no record (its parent is not expanded) or is the wrong kind.
    NotApplicable,
    /// Already in the requested state.
    AlreadyDone,
    /// The node or brick pool, or the brick ids, are exhausted.
    PoolFull,
    /// The node lies in a replaced chunk not applied yet: what the table
    /// holds there is about to be rebuilt.
    Waiting,
}

#[derive(Clone, Copy, Debug)]
struct BrickSlot {
    id: u32,
    alloc: BrickAlloc,
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    /// Where the node's record is in the node pool.
    index: u32,
    /// The record as last written.
    record: GpuNode,
    /// The child group, when expanded (level 2 and up).
    group: Option<NodeAlloc>,
    /// The resident block, for a level-1 node.
    brick: Option<BrickSlot>,
    /// The frame a ray last asked for this node.
    last_requested: u32,
    /// Slots of the children that have entries (mixed content), when
    /// expanded.
    kids: u64,
    /// The mask the group or the brick block was laid out for: the GPU
    /// finds children and leaves by their rank in the record's mask, so a
    /// record may only point at them with this mask.
    layout: u64,
    /// Slots of the children whose bricks are resident (level 2).
    bricks: u64,
    /// Slots of the children that are expanded.
    grown: u64,
    /// Slots of the expanded children with nothing to load: every child
    /// of theirs with an entry has its brick resident (see [`Entry::done`]).
    done: u64,
}

impl Entry {
    /// Expanded, with every child that has an entry holding its brick:
    /// stage 1 finds nothing to do below it.
    fn done(&self) -> bool {
        self.group.is_some() && self.kids & !self.bricks == 0
    }

    fn new(index: u32, record: GpuNode) -> Self {
        Self {
            index,
            record,
            group: None,
            brick: None,
            last_requested: 0,
            kids: 0,
            layout: 0,
            bricks: 0,
            grown: 0,
            done: 0,
        }
    }
}

/// Usage figures, for budgets and the debug overlay.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ResidencyStats {
    pub entries: usize,
    pub expanded: usize,
    pub bricks: usize,
    pub node_fill: f32,
    pub brick_fill: f32,
    pub brick_bytes: u64,
}

/// The residency table and the slot bookkeeping of the pools.
pub struct Residency {
    shape: Shape,
    entries: Table<Entry>,
    /// Cached summaries, those below the chunk level kept by chunk (split
    /// at [`chunk_level`](Self::chunk_level)), so a replaced chunk forgets
    /// its own at once.
    summaries: Table<Summary>,
    edited: HashSet<NodeKey>,
    /// Changes not applied yet, in arrival order without repeats.
    pending: Vec<Change>,
    /// Replaced chunks among them: their summaries are not computed until
    /// they are applied (see [`summary`](Self::summary)).
    pending_chunks: HashSet<NodeKey>,
    /// Waiting replaced chunks being summarized ahead of their application,
    /// a share of the budget per frame: the next grandchild to summarize,
    /// as `child slot * 64 + grandchild slot` (4096 when done).
    warming: HashMap<NodeKey, u32>,
    /// Node summaries computed so far, the unit of the change budget.
    summarized: u64,
    /// Summaries of replaced chunks computed where they were built, by
    /// chunk, until the chunk is applied (decision 0022). Each is checked
    /// against the source when used.
    offered: HashMap<NodeKey, ChunkSummaries>,
    /// Chunks applied with offered summaries, and offers refused because the
    /// source no longer held the content they describe.
    offers_used: u64,
    offers_stale: u64,
    /// Entries taken out of the tree by a collapse or a replaced chunk and
    /// not freed yet, with the node they were taken from: freeing a large
    /// subtree is spread over frames (see [`free_buried`](Self::free_buried)).
    /// Their descendants stay in the table under their keys until freed.
    graveyard: std::collections::VecDeque<(NodeKey, NodeKey, Entry)>,
    /// The nodes with descendants still in the graveyard or below it, and
    /// how many graveyard entries each has left. Nothing below them is
    /// refined until they are freed.
    burying: HashMap<NodeKey, u32>,
    nodes: NodeSpace,
    bricks: BrickSpace,
    ids: IdSpace,
    /// The level-1 node each brick id holds.
    id_keys: Vec<Option<NodeKey>>,
    frame: u32,
    tally: Tally,
    /// Tallies for summaries of branches, reused down the recursion.
    spare_tallies: Vec<Tally>,
    /// Stage 1's walk in progress: nodes still to visit (see
    /// [`select_budgeted`](super::select::select_budgeted)).
    walk: Vec<NodeKey>,
    /// Stage 1's growth pass in progress and carried candidates.
    growth: GrowthState,
    /// Stage 1's detail threshold multiplier under node pool pressure (see
    /// `frame::adapt_pressure`); 1 when the pool has room.
    pressure: f32,
    /// The pressure just changed and the shrink pass it started is still
    /// running: it walks at the growth budget, and the pressure holds until
    /// its collapses have landed.
    settling: bool,
    /// The frame the pressure last rose.
    pressure_raised_at: u32,
}

/// A change of the source the table has not caught up with yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Change {
    /// The content of a level-1 node (a brick) changed.
    Brick(NodeKey),
    /// The whole node at the chunk level was replaced (loaded, unloaded,
    /// or switched between full and coarse detail).
    Chunk(NodeKey),
}

impl Change {
    pub fn key(self) -> NodeKey {
        match self {
            Self::Brick(k) | Self::Chunk(k) => k,
        }
    }
}

/// What [`Residency::apply_changes`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    /// Changes applied.
    pub applied: u32,
    /// Changes still waiting.
    pub pending: u32,
    /// Node summaries computed while applying them.
    pub summarized: u64,
}

/// What stage 1 needs of one node, from a single table lookup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeState {
    pub expanded: bool,
    pub brick_resident: bool,
    /// Children with content, by slot.
    pub mask: u64,
    /// Children with records of their own (mixed content), by slot: the
    /// only ones that can need work.
    pub kids: u64,
    /// Children whose bricks are resident, by slot (level 2).
    pub bricks: u64,
    /// Expanded children, by slot.
    pub grown: u64,
    /// Expanded children with nothing to load, by slot: every child of
    /// theirs with an entry has its brick resident.
    pub done: u64,
}

impl Residency {
    /// An empty table for a tree of `shape` in pools of `layout`. The first
    /// [`reset`](Self::reset) writes the root.
    pub fn new(shape: Shape, layout: &PoolLayout) -> Option<Self> {
        Some(Self {
            shape,
            // Levels 1 and 2 kept by their level-3 ancestor.
            entries: Table::new(3),
            summaries: Table::new(world_core::tree::CHUNK_LEVEL.min(shape.root_level()).max(1)),
            edited: HashSet::default(),
            pending: Vec::new(),
            pending_chunks: HashSet::default(),
            warming: HashMap::default(),
            summarized: 0,
            offered: HashMap::default(),
            offers_used: 0,
            offers_stale: 0,
            graveyard: std::collections::VecDeque::new(),
            burying: HashMap::default(),
            nodes: NodeSpace::new(layout.node_count, layout.node_groups)?,
            bricks: BrickSpace::new(&layout.segments, layout.allocs_per_segment),
            ids: IdSpace::new(layout.brick_ids),
            id_keys: vec![None; layout.brick_ids as usize],
            frame: 1,
            tally: Tally::default(),
            spare_tallies: Vec::new(),
            walk: Vec::new(),
            growth: GrowthState::default(),
            pressure: 1.0,
            settling: false,
            pressure_raised_at: 0,
        })
    }

    pub fn shape(&self) -> Shape {
        self.shape
    }

    /// Sets the frame number stamped on uploads (1 or more; 0 marks a free
    /// brick id on the GPU).
    pub fn set_frame(&mut self, frame: u32) {
        self.frame = frame.max(1);
    }

    pub fn frame(&self) -> u32 {
        self.frame
    }

    /// O(1): every expanded node holds one node group and every resident
    /// brick one brick id ([`verify`](Self::verify) checks both).
    pub fn stats(&self) -> ResidencyStats {
        ResidencyStats {
            entries: self.entries.len(),
            expanded: self.nodes.groups() as usize,
            bricks: self.ids.in_use() as usize,
            node_fill: self.nodes.fill(),
            brick_fill: self.bricks.fill(),
            brick_bytes: self.bricks.used_bytes(),
        }
    }

    /// Entries in the largest of the maps the table is split into: what one
    /// insertion may have to rehash at worst.
    pub fn largest_table_part(&self) -> usize {
        self.entries.largest_part()
    }

    /// Resident bricks: each holds exactly one brick id. O(1), unlike
    /// [`stats`](Self::stats), which scans the table.
    pub fn brick_count(&self) -> usize {
        self.ids.in_use() as usize
    }

    /// One more than the largest brick id ever handed out: the eviction
    /// pass scans only the table below it.
    pub fn brick_id_high_water(&self) -> u32 {
        self.ids.high_water()
    }

    pub fn brick_fill(&self) -> f32 {
        self.bricks.fill()
    }

    pub fn node_fill(&self) -> f32 {
        self.nodes.fill()
    }

    pub fn has_record(&self, key: NodeKey) -> bool {
        self.entries.contains_key(&key)
    }

    /// The node's state, or `None` when it has no record.
    pub fn node_state(&self, key: NodeKey) -> Option<NodeState> {
        self.entries.get(&key).map(|e| NodeState {
            expanded: e.group.is_some(),
            brick_resident: e.brick.is_some(),
            mask: e.record.mask(),
            kids: e.kids,
            bricks: e.bricks,
            grown: e.grown,
            done: e.done,
        })
    }

    /// Takes stage 1's walk in progress, to resume it.
    pub fn take_walk(&mut self) -> Vec<NodeKey> {
        std::mem::take(&mut self.walk)
    }

    /// Puts back stage 1's walk in progress.
    pub fn put_walk(&mut self, walk: Vec<NodeKey>) {
        self.walk = walk;
    }

    /// Stage 1's growth pass and carried candidates.
    pub fn growth(&mut self) -> &mut GrowthState {
        &mut self.growth
    }

    /// Takes stage 1's growth state, to run the next part of its pass.
    pub fn take_growth(&mut self) -> GrowthState {
        std::mem::take(&mut self.growth)
    }

    /// Puts back stage 1's growth state.
    pub fn put_growth(&mut self, growth: GrowthState) {
        self.growth = growth;
    }

    /// Stage 1's detail threshold multiplier under node pool pressure.
    pub fn pressure(&self) -> f32 {
        self.pressure
    }

    /// Sets the pressure; a change starts a new shrink pass at once and
    /// holds the pressure until it is done (see [`settling`](Self::settling)).
    pub fn set_pressure(&mut self, pressure: f32) {
        if pressure != self.pressure {
            self.settling = true;
            self.walk.clear();
            self.growth.shrink_changed();
        }
        self.pressure = pressure;
    }

    /// The frame the pressure last rose.
    pub fn pressure_raised_at(&self) -> u32 {
        self.pressure_raised_at
    }

    pub fn set_pressure_raised_at(&mut self, frame: u32) {
        self.pressure_raised_at = frame;
    }

    /// Whether the shrink pass a pressure change started is still running.
    pub fn settling(&self) -> bool {
        self.settling
    }

    /// The shrink pass a pressure change started is done.
    pub fn settled(&mut self) {
        self.settling = false;
    }

    pub fn is_expanded(&self, key: NodeKey) -> bool {
        self.entries.get(&key).is_some_and(|e| e.group.is_some())
    }

    pub fn is_brick_resident(&self, key: NodeKey) -> bool {
        self.entries.get(&key).is_some_and(|e| e.brick.is_some())
    }

    pub fn record(&self, key: NodeKey) -> Option<GpuNode> {
        self.entries.get(&key).map(|e| e.record)
    }

    /// The pool index of a node's record.
    pub fn record_index(&self, key: NodeKey) -> Option<u32> {
        self.entries.get(&key).map(|e| e.index)
    }

    /// The level-1 nodes whose bricks are resident, sorted.
    pub fn resident_bricks(&self) -> Vec<NodeKey> {
        let mut v: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, e)| e.brick.is_some())
            .map(|(k, _)| k)
            .collect();
        v.sort_unstable();
        v
    }

    /// Every node with a record, sorted.
    pub fn keys(&self) -> Vec<NodeKey> {
        let mut v: Vec<_> = self.entries.keys().collect();
        v.sort_unstable();
        v
    }

    /// The children of `key` that have records, in slot order.
    pub fn children_with_records(&self, key: NodeKey) -> Vec<NodeKey> {
        let Some(e) = self.entries.get(&key) else {
            return Vec::new();
        };
        if e.group.is_none() {
            return Vec::new();
        }
        let mask = e.record.mask();
        (0..64)
            .filter(|&s| mask & (1u64 << s) != 0)
            .map(|s| key.child(s))
            .filter(|c| self.entries.contains_key(c))
            .collect()
    }

    /// Remembers that a ray asked for `key` this frame.
    pub fn note_request(&mut self, key: NodeKey) {
        let frame = self.frame;
        if let Some(e) = self.entries.get_mut(&key) {
            e.last_requested = frame;
        }
    }

    pub fn last_requested(&self, key: NodeKey) -> u32 {
        self.entries.get(&key).map_or(0, |e| e.last_requested)
    }

    /// The node a brick id holds, if the id is in use.
    pub fn key_of_id(&self, id: u32) -> Option<NodeKey> {
        self.id_keys.get(id as usize).copied().flatten()
    }

    /// The block ref a brick id currently points at.
    pub fn block_of_id(&self, id: u32) -> Option<BrickRef> {
        let key = self.key_of_id(id)?;
        let slot = self.entries.get(&key)?.brick?;
        (slot.id == id).then_some(slot.alloc.block)
    }

    // ----------------------------------------------------------- summaries --

    fn edited_bit(&self, key: NodeKey) -> bool {
        // Nothing is edited until the player edits: no hashing until then.
        !self.edited.is_empty() && self.edited.contains(&key)
    }

    /// The level of the chunks: summaries below it are kept per chunk.
    pub fn chunk_level(&self) -> u32 {
        world_core::tree::CHUNK_LEVEL.min(self.shape.root_level())
    }

    fn cached_summary(&self, key: NodeKey) -> Option<Summary> {
        self.summaries.get(&key).copied()
    }

    fn cache_summary(&mut self, key: NodeKey, s: Summary) {
        self.summaries.insert(key, s);
    }

    /// Forgets the cached summary of `key`; whether there was one.
    fn forget_summary(&mut self, key: NodeKey) -> bool {
        self.summaries.remove(&key).is_some()
    }

    /// Node summaries computed since the table was made: a count of work,
    /// for budgets and tests.
    pub fn summaries_computed(&self) -> u64 {
        self.summarized
    }

    /// Takes the summaries of replaced chunks computed where they were built
    /// (decision 0022), to use when their chunks are applied instead of
    /// computing them. Offer them with the feed that reports the chunks:
    /// only offers for chunks waiting to be applied are kept (a later offer
    /// for the same chunk replaces an earlier one), and each is used only if
    /// the source then holds exactly the content it describes.
    pub fn offer_summaries(&mut self, offers: impl IntoIterator<Item = ChunkSummaries>) {
        let split = self.chunk_level();
        if split != WARMED_LEVEL {
            return;
        }
        for offer in offers {
            if let Some(local) = self.shape.local(offer.min()) {
                self.offered
                    .insert(NodeKey::containing(local, split), offer);
            }
        }
    }

    /// Chunks applied with offered summaries, and offers refused because
    /// the source had changed since (an edit, a newer replacement).
    pub fn offers_used(&self) -> (u64, u64) {
        (self.offers_used, self.offers_stale)
    }

    /// Offers held for chunks not applied yet.
    pub fn offers_held(&self) -> usize {
        self.offered.len()
    }

    /// The summary of the node `key`, whose view is `view`; cached for
    /// branches until a change below them invalidates it. A replaced chunk
    /// that is not applied yet and has no cached summary gets a stand-in
    /// (see [`stand_in`]) instead of a walk over its whole content: the
    /// records above it are rewritten when it is applied.
    fn summary<N: NodeView>(&mut self, key: NodeKey, view: N) -> Summary {
        // Only branches are cached: the others answer without a lookup.
        let level = match content(&view) {
            Content::Air | Content::Unloaded => return Summary::default(),
            Content::Uniform(m) => {
                return Summary {
                    material: m,
                    solid: node_volume(key.level()),
                };
            }
            Content::Leaf => return exact_summary(view, 0),
            Content::Branch => key.level(),
        };
        if let Some(s) = self.cached_summary(key) {
            return s;
        }
        if level == self.chunk_level() && self.pending_chunks.contains(&key) {
            return stand_in(key, view);
        }
        let s = if level <= 1 {
            // The scratch tally: no allocation per brick.
            self.tally.clear();
            count_voxels(view, 1, &mut self.tally);
            self.tally.summary()
        } else {
            let mut tally = self.spare_tallies.pop().unwrap_or_default();
            tally.clear();
            for (slot, child) in view.children() {
                let c = self.summary(key.child(slot), child);
                tally.add(c.material, c.solid);
            }
            let s = tally.summary();
            self.spare_tallies.push(tally);
            s
        };
        self.summarized += 1;
        self.cache_summary(key, s);
        s
    }

    /// The record a child should have, given the table, or `None` for air.
    fn child_record<N: NodeView>(&mut self, key: NodeKey, view: N) -> Option<GpuNode> {
        self.record_of(key, view, false)
    }

    /// The record to write for child `key` (whose view is `view`) when its
    /// parent's group is written: [`child_record`](Self::child_record),
    /// except that a mixed child whose group or brick block was laid out for
    /// another mask, or that lies in a replaced chunk that waits, keeps its
    /// current record. Such a node's content changed under a change not
    /// applied yet; until that change rewrites its path it keeps describing
    /// the old content (decision 0012), with the mask its group or block was
    /// laid out for. Rewriting it would make the GPU read children by rank
    /// in the wrong mask; dropping what it holds instead would throw away
    /// whole subtrees for stage 1 to rebuild.
    fn group_record<N: NodeView>(&mut self, key: NodeKey, view: N) -> Option<GpuNode> {
        self.record_of(key, view, true)
    }

    fn record_of<N: NodeView>(&mut self, key: NodeKey, view: N, keep: bool) -> Option<GpuNode> {
        let edited = self.edited_bit(key);
        match content(&view) {
            Content::Air => None,
            Content::Uniform(m) => Some(GpuNode::uniform(m, edited)),
            Content::Unloaded => Some(GpuNode::not_resident(0, MaterialId::AIR, edited)),
            Content::Branch | Content::Leaf => {
                let mask = view.mask();
                if keep
                    && let Some(e) = self.entries.get(&key)
                    && (e.group.is_some() || e.brick.is_some())
                    && (e.layout != mask || self.in_waiting_chunk(key))
                {
                    return Some(e.record);
                }
                let material = self.summary(key, view).material;
                let e = self.entries.get(&key);
                Some(match (e.and_then(|e| e.group), e.and_then(|e| e.brick)) {
                    (Some(g), _) if key.level >= 2 => {
                        GpuNode::internal(mask, g.first, material, edited)
                    }
                    (_, Some(b)) if key.level == 1 => {
                        GpuNode::brick(mask, b.alloc.block, material, edited)
                    }
                    _ => GpuNode::not_resident(mask, material, edited),
                })
            }
        }
    }

    // ---------------------------------------------------------- structure --

    /// Adds or updates the entry of `key`, keeping its parent's child mask.
    fn put_entry(&mut self, key: NodeKey, index: u32, record: GpuNode) {
        match self.entries.get_mut(&key) {
            Some(e) => {
                e.index = index;
                e.record = record;
            }
            None => {
                self.entries.insert(key, Entry::new(index, record));
            }
        }
        let mut flip = None;
        if key != self.shape.root()
            && let Some(p) = self.entries.get_mut(&key.parent())
        {
            let was = p.done();
            p.kids |= 1u64 << key.slot_in_parent();
            flip = (p.done() != was).then_some(!was);
        }
        if let Some(done) = flip {
            self.set_done_bit(key.parent(), done);
        }
    }

    /// Adds `kids` to the children with entries of `key`, as
    /// [`put_entry`](Self::put_entry) does for each child it inserts.
    fn add_kids(&mut self, key: NodeKey, kids: u64) {
        let mut flip = None;
        if let Some(e) = self.entries.get_mut(&key) {
            let was = e.done();
            e.kids |= kids;
            flip = (e.done() != was).then_some(!was);
        }
        if let Some(done) = flip {
            self.set_done_bit(key, done);
        }
    }

    /// Removes the entry of `key`, keeping its parent's child masks.
    fn remove_entry(&mut self, key: NodeKey) -> Option<Entry> {
        let e = self.entries.remove(&key)?;
        let mut flip = None;
        if key != self.shape.root()
            && let Some(p) = self.entries.get_mut(&key.parent())
        {
            let was = p.done();
            let bit = 1u64 << key.slot_in_parent();
            p.kids &= !bit;
            p.bricks &= !bit;
            p.grown &= !bit;
            p.done &= !bit;
            flip = (p.done() != was).then_some(!was);
        }
        if let Some(done) = flip {
            self.set_done_bit(key.parent(), done);
        }
        Some(e)
    }

    /// Sets whether expanded `key` has nothing to load, in its parent's
    /// mask. Only an expanded parent keeps these masks: the entries below a
    /// buried node stay in the table until they are freed, and the parent
    /// may meanwhile be a new, unexpanded entry with the same key.
    fn set_done_bit(&mut self, key: NodeKey, done: bool) {
        if key != self.shape.root()
            && let Some(p) = self.entries.get_mut(&key.parent())
            && p.group.is_some()
        {
            let bit = 1u64 << key.slot_in_parent();
            if done {
                p.done |= bit;
            } else {
                p.done &= !bit;
            }
        }
    }

    /// Sets the child group of `key`, keeping its parent's masks (only an
    /// expanded parent keeps them, see [`set_done_bit`](Self::set_done_bit)).
    fn set_group(&mut self, key: NodeKey, group: Option<NodeAlloc>) {
        let Some(e) = self.entries.get_mut(&key) else {
            return;
        };
        e.group = group;
        let (grown, done) = (group.is_some(), e.done());
        if key != self.shape.root()
            && let Some(p) = self.entries.get_mut(&key.parent())
            && p.group.is_some()
        {
            let bit = 1u64 << key.slot_in_parent();
            p.grown = if grown { p.grown | bit } else { p.grown & !bit };
            p.done = if done { p.done | bit } else { p.done & !bit };
        }
    }

    /// Sets the resident brick of level-1 node `key`, keeping its parent's
    /// brick mask.
    fn set_brick(&mut self, key: NodeKey, slot: Option<BrickSlot>) {
        let Some(e) = self.entries.get_mut(&key) else {
            return;
        };
        e.brick = slot;
        let mut flip = None;
        if key != self.shape.root()
            && let Some(p) = self.entries.get_mut(&key.parent())
        {
            let was = p.done();
            let bit = 1u64 << key.slot_in_parent();
            if slot.is_some() {
                p.bricks |= bit;
            } else {
                p.bricks &= !bit;
            }
            flip = (p.done() != was).then_some(!was);
        }
        if let Some(done) = flip {
            self.set_done_bit(key.parent(), done);
        }
    }

    /// Records the mask the group or brick block of `key` was laid out for.
    fn set_layout(&mut self, key: NodeKey, mask: u64) {
        if let Some(e) = self.entries.get_mut(&key) {
            e.layout = mask;
        }
    }

    /// Forgets everything below and including `key` without writing records:
    /// the caller rewrites the group or record that referred to it.
    fn drop_subtree(&mut self, key: NodeKey, batch: &mut UploadBatch) {
        let Some(e) = self.remove_entry(key) else {
            return;
        };
        self.release(key, e, batch);
    }

    /// Frees what an entry holds: its children (recursively), group and brick.
    fn release(&mut self, key: NodeKey, e: Entry, batch: &mut UploadBatch) {
        if let Some(g) = e.group {
            // The entries below, not the record's mask: a record may have
            // been rewritten for content the group does not hold yet.
            let mut kids = e.kids;
            while kids != 0 {
                let slot = kids.trailing_zeros();
                kids &= kids - 1;
                self.drop_subtree(key.child(slot), batch);
            }
            self.nodes.free(g);
        }
        if let Some(b) = e.brick {
            self.free_brick(b, batch);
        }
    }

    fn free_brick(&mut self, b: BrickSlot, batch: &mut UploadBatch) {
        self.bricks.free(b.alloc);
        self.ids.free(b.id);
        if let Some(k) = self.id_keys.get_mut(b.id as usize) {
            *k = None;
        }
        batch.write_table(b.id, None, 0);
    }

    fn write_record(&mut self, key: NodeKey, record: GpuNode, batch: &mut UploadBatch) {
        if let Some(e) = self.entries.get_mut(&key) {
            e.record = record;
            batch.write_node(e.index, record);
        }
    }

    /// Writes the child group of `key` (whose view is `view`, a branch) from
    /// the source and the table, reallocating it when the child count
    /// changed. On a full node pool the node is collapsed instead.
    fn write_group<N: NodeView>(
        &mut self,
        key: NodeKey,
        view: N,
        batch: &mut UploadBatch,
    ) -> Result<(), Refused> {
        let mask = view.mask();
        let n = mask.count_ones();
        // Children that became air or uniform lose their entries. The
        // children with entries are exactly `kids` (verify checks it), so a
        // node expanded afresh looks none up.
        let mut kids = self.entries.get(&key).map_or(0, |e| e.kids);
        while kids != 0 {
            let slot = kids.trailing_zeros();
            kids &= kids - 1;
            let gone = mask & (1u64 << slot) == 0 || content(&view.child(slot)) != Content::Branch;
            if gone {
                self.drop_subtree(key.child(slot), batch);
            }
        }

        let old = self.entries.get(&key).and_then(|e| e.group);
        let group = match old {
            Some(g) if g.len == n.max(1) => g,
            other => {
                if let Some(g) = other {
                    self.nodes.free(g);
                }
                match self.nodes.allocate(n) {
                    Some(g) => g,
                    None => {
                        // Give up the subtree rather than leave it stale.
                        self.set_group(key, None);
                        let children: Vec<u32> = (0..64).collect();
                        for slot in children {
                            self.drop_subtree(key.child(slot), batch);
                        }
                        return Err(Refused::PoolFull);
                    }
                }
            }
        };
        self.set_group(key, Some(group));
        let fresh = match self.entries.get_mut(&key) {
            Some(e) => {
                e.layout = mask;
                e.kids == 0
            }
            None => false,
        };

        if fresh {
            // No child has an entry (those with one are exactly `kids`), so
            // none keeps its record or holds a group or brick: each record
            // is what `group_record` gives a child without an entry, the
            // entries go in in slot order, and the masks are set once.
            let mut kids = 0u64;
            for (k, (slot, child)) in view.children().enumerate() {
                let ck = key.child(slot);
                let index = group.first + k as u32;
                let edited = self.edited_bit(ck);
                let record = match content(&child) {
                    Content::Air => continue,
                    Content::Uniform(m) => GpuNode::uniform(m, edited),
                    Content::Unloaded => GpuNode::not_resident(0, MaterialId::AIR, edited),
                    c @ (Content::Branch | Content::Leaf) => {
                        let material = self.summary(ck, child).material;
                        let record = GpuNode::not_resident(child.mask(), material, edited);
                        if c == Content::Branch {
                            let old = self.entries.insert(ck, Entry::new(index, record));
                            debug_assert!(old.is_none(), "{ck:?} had an entry outside kids");
                            kids |= 1u64 << slot;
                        }
                        record
                    }
                };
                batch.write_node(index, record);
            }
            self.add_kids(key, kids);
            return Ok(());
        }
        for (k, (slot, child)) in view.children().enumerate() {
            let ck = key.child(slot);
            let index = group.first + k as u32;
            let Some(record) = self.group_record(ck, child) else {
                continue;
            };
            if content(&child) == Content::Branch {
                self.put_entry(ck, index, record);
            }
            batch.write_node(index, record);
        }
        Ok(())
    }

    /// Rewrites the records of the children of expanded node `key` (whose
    /// view is `view`, a branch) in `slots`, which are the only ones whose
    /// content may have changed. When its mask changed, the group's layout
    /// did too, and the whole group is written again.
    fn rewrite_children<N: NodeView>(
        &mut self,
        key: NodeKey,
        view: N,
        slots: u64,
        batch: &mut UploadBatch,
    ) -> Result<(), Refused> {
        let Some(e) = self.entries.get(&key) else {
            return Ok(());
        };
        let Some(group) = e.group else {
            return Ok(());
        };
        let mask = view.mask();
        if slots == u64::MAX || mask != e.layout || group.len != mask.count_ones().max(1) {
            return self.write_group(key, view, batch);
        }
        // A changed slot that became air changed the mask: all are set.
        let mut todo = slots & mask;
        while todo != 0 {
            let slot = todo.trailing_zeros();
            todo &= todo - 1;
            let ck = key.child(slot);
            let child = view.child(slot);
            let branch = content(&child) == Content::Branch;
            if !branch && self.entries.contains_key(&ck) {
                self.drop_subtree(ck, batch);
            }
            let index = group.first + (mask & ((1u64 << slot) - 1)).count_ones();
            if let Some(record) = self.group_record(ck, child) {
                if branch {
                    self.put_entry(ck, index, record);
                }
                batch.write_node(index, record);
            }
        }
        Ok(())
    }

    /// Rewrites the root record from the source, by the rule of
    /// [`group_record`](Self::group_record): a root whose group is still
    /// laid out for another mask (nothing on its path was rewritten, as
    /// when the only change applied was folded into a waiting chunk) keeps
    /// its record.
    fn write_root<S: VoxelSource>(&mut self, source: &S, batch: &mut UploadBatch) {
        let root = self.shape.root();
        let view = source.root();
        let record = match content(&view) {
            Content::Air => GpuNode::internal(0, 0, MaterialId::AIR, false),
            _ => self.group_record(root, view).unwrap_or(GpuNode::internal(
                0,
                0,
                MaterialId::AIR,
                false,
            )),
        };
        match self.entries.get_mut(&root) {
            Some(e) => e.record = record,
            None => {
                self.entries.insert(root, Entry::new(ROOT_NODE, record));
            }
        }
        batch.write_node(ROOT_NODE, record);
    }

    /// Drops everything and writes the root from `source`, not expanded.
    pub fn reset<S: VoxelSource>(&mut self, source: &S, batch: &mut UploadBatch) {
        self.free_buried(u32::MAX, batch);
        let root = self.shape.root();
        if let Some(e) = self.entries.remove(&root) {
            self.release(root, e, batch);
        }
        self.entries.clear();
        self.summaries.clear();
        self.pending.clear();
        self.pending_chunks.clear();
        self.warming.clear();
        self.offered.clear();
        self.walk.clear();
        self.growth.changed();
        self.write_root(source, batch);
    }

    /// Makes the children of `key` (level 2 or up) resident as records.
    /// Costs `popcount(mask)` node records.
    pub fn expand<S: VoxelSource>(
        &mut self,
        source: &S,
        key: NodeKey,
        batch: &mut UploadBatch,
    ) -> Result<u32, Refused> {
        let Some(e) = self.entries.get(&key) else {
            return Err(Refused::NotApplicable);
        };
        if key.level < 2 {
            return Err(Refused::NotApplicable);
        }
        if e.group.is_some() {
            return Err(Refused::AlreadyDone);
        }
        if self.is_blocked(key) {
            return Err(Refused::Waiting);
        }
        let view = view_at(source, key);
        if content(&view) != Content::Branch {
            return Err(Refused::NotApplicable);
        }
        self.write_group(key, view, batch)?;
        let record = if key == self.shape.root() {
            self.write_root(source, batch);
            return Ok(view.mask().count_ones());
        } else {
            self.child_record(key, view)
        };
        if let Some(record) = record {
            self.write_record(key, record, batch);
        }
        Ok(view.mask().count_ones())
    }

    /// Takes the children of `key` off the GPU (recursively, bricks
    /// included). Its record becomes not resident with the same mask and
    /// material.
    pub fn collapse(&mut self, key: NodeKey, batch: &mut UploadBatch) -> Result<(), Refused> {
        let Some(&e) = self.entries.get(&key) else {
            return Err(Refused::NotApplicable);
        };
        if e.group.is_none() {
            return Err(Refused::AlreadyDone);
        }
        // A leftover of a burial is freed by the graveyard, not here.
        if self.in_buried(key) {
            return Err(Refused::Waiting);
        }
        self.bury_children(key);
        if let Some(g) = e.group {
            self.nodes.free(g);
        }
        self.set_group(key, None);
        self.write_record(key, e.record.unlinked(), batch);
        Ok(())
    }

    /// Takes the children of `key` out of the tree into the graveyard:
    /// their groups and bricks, and their descendants, are freed later
    /// under a budget. The caller rewrites or frees the group that held
    /// their records.
    fn bury_children(&mut self, key: NodeKey) {
        let Some(e) = self.entries.get(&key) else {
            return;
        };
        let mut kids = e.kids;
        while kids != 0 {
            let slot = kids.trailing_zeros();
            kids &= kids - 1;
            let ck = key.child(slot);
            if let Some(ce) = self.remove_entry(ck) {
                self.graveyard.push_back((key, ck, ce));
                *self.burying.entry(key).or_default() += 1;
            }
        }
    }

    /// Frees buried entries (their groups and bricks), oldest first, taking
    /// their children out of the table into the graveyard in turn, until
    /// `max` entries were freed or taken out (at most 64 more). Returns how
    /// many were freed. A node is no longer burying once everything below
    /// it is freed, and stage 1 then starts a new pass, since its passes
    /// skipped what lies below it.
    pub fn free_buried(&mut self, max: u32, batch: &mut UploadBatch) -> u32 {
        let mut freed = 0;
        let mut work = 0u32;
        while work < max {
            // Oldest first, so no burial waits behind newer ones.
            let Some((tag, key, e)) = self.graveyard.pop_front() else {
                break;
            };
            if let Some(g) = e.group {
                let mut kids = e.kids;
                while kids != 0 {
                    let slot = kids.trailing_zeros();
                    kids &= kids - 1;
                    let ck = key.child(slot);
                    // Its parent left the table already: no masks to keep.
                    if let Some(ce) = self.entries.remove(&ck) {
                        self.graveyard.push_back((tag, ck, ce));
                        *self.burying.entry(tag).or_default() += 1;
                        work = work.saturating_add(1);
                    }
                }
                self.nodes.free(g);
            }
            if let Some(b) = e.brick {
                self.free_brick(b, batch);
            }
            if let Some(left) = self.burying.get_mut(&tag) {
                *left -= 1;
                if *left == 0 {
                    // Stage 1 skipped everything below it: look again.
                    self.burying.remove(&tag);
                    self.growth.changed();
                }
            }
            freed += 1;
            work = work.saturating_add(1);
        }
        freed
    }

    /// Entries in the graveyard, not counting their descendants.
    pub fn buried(&self) -> usize {
        self.graveyard.len()
    }

    /// Whether `key` lies below a node whose descendants are still being
    /// freed: its table entry may be one of them.
    pub fn in_buried(&self, key: NodeKey) -> bool {
        !self.burying.is_empty()
            && (key.level()..self.shape.root_level())
                .any(|level| self.burying.contains_key(&key.ancestor(level + 1)))
    }

    /// Whether stage 1 must leave `key` alone: it lies in a replaced chunk
    /// that waits, or below a node whose descendants are being freed.
    pub fn is_blocked(&self, key: NodeKey) -> bool {
        self.in_waiting_chunk(key) || self.in_buried(key)
    }

    /// Uploads the brick of level-1 node `key`. Returns the block's bytes.
    pub fn load_brick<S: VoxelSource>(
        &mut self,
        source: &S,
        key: NodeKey,
        batch: &mut UploadBatch,
    ) -> Result<u64, Refused> {
        let Some(e) = self.entries.get(&key) else {
            return Err(Refused::NotApplicable);
        };
        if key.level != 1 {
            return Err(Refused::NotApplicable);
        }
        if e.brick.is_some() {
            return Err(Refused::AlreadyDone);
        }
        if self.is_blocked(key) {
            return Err(Refused::Waiting);
        }
        let view = view_at(source, key);
        if content(&view) != Content::Branch {
            return Err(Refused::NotApplicable);
        }
        let slot = self.place_brick(key, view, batch)?;
        let edited = self.edited_bit(key);
        let material = self.summary(key, view).material;
        let record = GpuNode::brick(view.mask(), slot.alloc.block, material, edited);
        self.set_brick(key, Some(slot));
        self.set_layout(key, view.mask());
        self.write_record(key, record, batch);
        Ok(u64::from(slot.alloc.units) * 16)
    }

    /// Loads several bricks, encoding them on up to `threads` threads, in
    /// the order given, while `budget_left` says the batch has room. Ids are
    /// handed out in order before encoding, so the result is exactly what
    /// calling [`load_brick`](Self::load_brick) for each key gives. Returns
    /// how many bricks were loaded and whether the pools filled up.
    pub fn load_bricks<S: VoxelSource + Sync>(
        &mut self,
        source: &S,
        keys: &[NodeKey],
        threads: usize,
        mut budget_left: impl FnMut(&UploadBatch) -> bool,
        batch: &mut UploadBatch,
    ) -> (u32, bool) {
        // What may load: level-1 records without a brick. Whether the content
        // is mixed is decided by the encoder threads, which read it anyway.
        let mut jobs: Vec<(NodeKey, u32, bool)> = Vec::new();
        for &key in keys {
            let ok = key.level == 1
                && self.entries.get(&key).is_some_and(|e| e.brick.is_none())
                && !self.is_blocked(key);
            if !ok {
                continue;
            }
            let Some(id) = self.ids.allocate() else {
                break;
            };
            jobs.push((key, id, self.edited_bit(key)));
        }
        let encoded = encode_parallel(source, &jobs, threads.max(1));

        let mut loaded = 0;
        let mut full = false;
        let mut used = 0;
        for ((key, id, edited), block) in jobs.iter().zip(encoded) {
            if full || !budget_left(batch) {
                break;
            }
            used += 1;
            let Some((words, encoded)) = block else {
                // Air or one material: no block, the record says it all.
                self.ids.free(*id);
                continue;
            };
            let Some(alloc) = self.bricks.allocate(encoded.words) else {
                full = true;
                self.ids.free(*id);
                continue;
            };
            let start = batch.data_mut().len();
            batch.data_mut().extend_from_slice(&words);
            self.cache_summary(*key, encoded.summary);
            batch.scatter(start as u32, alloc.block, encoded.words);
            batch.write_table(*id, Some(alloc.block), self.frame);
            if let Some(k) = self.id_keys.get_mut(*id as usize) {
                *k = Some(*key);
            }
            let slot = BrickSlot { id: *id, alloc };
            let record =
                GpuNode::brick(encoded.mask, alloc.block, encoded.summary.material, *edited);
            self.set_brick(*key, Some(slot));
            self.set_layout(*key, encoded.mask);
            self.write_record(*key, record, batch);
            loaded += 1;
        }
        // Ids handed out for bricks the budget or the pool turned away.
        for (_, id, _) in &jobs[used..] {
            self.ids.free(*id);
        }
        (loaded, full)
    }

    /// Encodes the brick into the batch, allocates its block and id, and
    /// records the scatter and table write. The record is the caller's job.
    fn place_brick<N: NodeView>(
        &mut self,
        key: NodeKey,
        view: N,
        batch: &mut UploadBatch,
    ) -> Result<BrickSlot, Refused> {
        let id = self.ids.allocate().ok_or(Refused::PoolFull)?;
        let edited = self.edited_bit(key);
        let start = batch.data_mut().len();
        let encoded = encode_brick(view, id, edited, batch.data_mut(), &mut self.tally);
        let Some(alloc) = self.bricks.allocate(encoded.words) else {
            batch.data_mut().truncate(start);
            self.ids.free(id);
            return Err(Refused::PoolFull);
        };
        self.cache_summary(key, encoded.summary);
        batch.scatter(start as u32, alloc.block, encoded.words);
        batch.write_table(id, Some(alloc.block), self.frame);
        if let Some(k) = self.id_keys.get_mut(id as usize) {
            *k = Some(key);
        }
        Ok(BrickSlot { id, alloc })
    }

    /// Takes the brick of `key` off the GPU; its record keeps mask and
    /// material.
    pub fn evict_brick(&mut self, key: NodeKey, batch: &mut UploadBatch) -> Result<(), Refused> {
        let Some(e) = self.entries.get(&key) else {
            return Err(Refused::NotApplicable);
        };
        let Some(slot) = e.brick else {
            return Err(Refused::AlreadyDone);
        };
        if self.in_buried(key) {
            return Err(Refused::Waiting);
        }
        let record = e.record.unlinked();
        self.set_brick(key, None);
        self.free_brick(slot, batch);
        self.write_record(key, record, batch);
        Ok(())
    }

    /// Evicts the brick an eviction pass chose, if the id still holds the
    /// block it was chosen with (the readback is frames old).
    pub fn evict_victim(&mut self, id: u32, block: u32, batch: &mut UploadBatch) -> bool {
        match (self.key_of_id(id), self.block_of_id(id)) {
            (Some(key), Some(current)) if current.pack() == block && !self.in_buried(key) => {
                self.evict_brick(key, batch).is_ok()
            }
            _ => false,
        }
    }

    /// Applies an edit batch's changed bricks at once (see
    /// [`apply_changes`](Self::apply_changes)), after freeing everything
    /// buried, which an edit below would otherwise wait for.
    pub fn apply_edits<S: VoxelSource + Sync>(
        &mut self,
        source: &S,
        dirty: &[NodeKey],
        reupload: impl FnMut(u64) -> bool,
        batch: &mut UploadBatch,
    ) {
        self.free_buried(u32::MAX, batch);
        self.queue_changes(dirty.iter().map(|&k| Change::Brick(k)));
        self.apply_changes(source, None, u64::MAX, 1, reupload, batch);
    }

    /// Queues changes of the source; [`apply_changes`](Self::apply_changes)
    /// applies them. A change already waiting is not queued twice, but a
    /// chunk replaced again starts its summaries over.
    pub fn queue_changes(&mut self, changes: impl IntoIterator<Item = Change>) {
        let mut seen: HashSet<Change> = self.pending.iter().copied().collect();
        for c in changes {
            if let Change::Chunk(k) = c {
                self.warming.remove(&k);
                self.pending_chunks.insert(k);
            }
            if seen.insert(c) {
                self.pending.push(c);
            }
        }
    }

    /// Changes queued and not applied yet.
    pub fn pending_changes(&self) -> usize {
        self.pending.len()
    }

    /// Whether `key` lies in (or is) a replaced chunk not applied yet.
    /// Expanding or loading there is wasted: applying the chunk drops it,
    /// and summarizing what its old content cached can cost a walk over
    /// the whole chunk.
    pub fn in_waiting_chunk(&self, key: NodeKey) -> bool {
        let split = self.chunk_level();
        !self.pending_chunks.is_empty()
            && key.level() <= split
            && self.pending_chunks.contains(&key.ancestor(split))
    }

    /// Applies queued changes, nearest to `eye` first (in key order without
    /// one), until they have computed `max_summaries` node summaries; a
    /// changed brick is charged the cached summaries on its path that it
    /// forgets (at least one), since rewriting the path computes them again
    /// after the others are chosen. A replaced chunk is summarized first, a
    /// grandchild at a time and across calls if the budget runs out (at
    /// least one grandchild per call), and applied once that is done; an
    /// edit inside it meanwhile only marks its path edited, since applying
    /// the chunk rebuilds everything below it. A replaced chunk whose
    /// summaries were offered ([`offer_summaries`](Self::offer_summaries))
    /// and still describe the source skips the summarizing: it is charged
    /// [`OFFER_CHARGE`] and applied at once. At least one change makes
    /// progress per call. The rest wait for the next call, and until then
    /// the records above them describe the content they had before.
    ///
    /// The budget can be exceeded by the last change it lets in: a chunk's
    /// planned grandchildren stop once they reach it, so by at most one
    /// grandchild (65 summaries), plus the children whose grandchildren
    /// that completes (up to 64) and the chunk itself when it is applied
    /// (an offered chunk by its charge and the chunk itself);
    /// a changed brick by its path (up to the tree's depth); and the path
    /// rewrites that follow.
    ///
    /// A changed brick: `reupload` decides, in that order, which resident
    /// changed bricks are uploaded again (it gets the block bytes the brick
    /// needs and returns whether it fits the budget); the rest become not
    /// resident. A replaced chunk drops everything resident below it (it
    /// described the old content) and is summarized afresh. Then every
    /// record on the applied changes' paths is rewritten from the source.
    pub fn apply_changes<S: VoxelSource + Sync>(
        &mut self,
        source: &S,
        eye: Option<[f32; 3]>,
        max_summaries: u64,
        threads: usize,
        mut reupload: impl FnMut(u64) -> bool,
        batch: &mut UploadBatch,
    ) -> Applied {
        let start = self.summarized;
        if !self.offered.is_empty() {
            let waiting = &self.pending_chunks;
            self.offered.retain(|k, _| waiting.contains(k));
        }
        let mut order = std::mem::take(&mut self.pending);
        if let Some(eye) = eye {
            let origin = self.shape.origin;
            // Distances are never negative, so their bits sort like them.
            order.sort_by_cached_key(|c| {
                let d = super::select::distance(origin, c.key(), eye);
                (d.to_bits(), c.key())
            });
        } else {
            order.sort_by_key(|c| c.key());
        }
        let root_level = self.shape.root_level();
        // Expanded ancestors of the applied changes, bottom-up, with the
        // slots of their children on the changed paths (all of them for a
        // replaced chunk's own group).
        let mut groups: std::collections::BTreeMap<NodeKey, u64> = Default::default();
        let mut applied = 0usize;
        let mut worked = false;
        let mut rest = Vec::new();
        // Summaries a changed brick's path will need again: they are
        // computed after the loop, when the paths are rewritten, so they are
        // charged here.
        let mut charged = 0u64;
        for change in order {
            let spent = self.summarized - start + charged;
            if worked && spent >= max_summaries {
                rest.push(change);
                continue;
            }
            worked = true;
            let done = match change {
                Change::Brick(k) if self.in_buried(k) => {
                    // Its old entries are still being freed: it applies
                    // once they are.
                    rest.push(change);
                    continue;
                }
                Change::Brick(k) if self.in_waiting_chunk(k) => {
                    // Its chunk is rebuilt when applied; only the edited
                    // marks carry over.
                    self.mark_edited(k);
                    applied += 1;
                    continue;
                }
                Change::Brick(k) => {
                    charged += self.apply_brick(source, k, &mut reupload, batch).max(1);
                    true
                }
                Change::Chunk(c) => {
                    let ready = if self.take_offer(source, c) {
                        charged += OFFER_CHARGE;
                        true
                    } else {
                        self.warm_chunk(source, c, max_summaries.saturating_sub(spent), threads)
                    };
                    if ready {
                        self.apply_chunk(source, c);
                    }
                    ready
                }
            };
            if !done {
                rest.push(change);
                continue;
            }
            applied += 1;
            let k = change.key();
            for level in k.level().max(2)..=root_level {
                let a = k.ancestor(level);
                if self.is_expanded(a) {
                    let slots = if a == k {
                        u64::MAX
                    } else {
                        1u64 << k.ancestor(level - 1).slot_in_parent()
                    };
                    *groups.entry(a).or_default() |= slots;
                }
            }
        }
        self.pending = rest;

        // Every expanded ancestor rewrites the records of its children on
        // the changed paths, bottom-up (its whole group when its mask
        // changed).
        for (a, slots) in groups {
            if !self.entries.contains_key(&a) {
                continue; // dropped by a lower rewrite
            }
            let view = view_at(source, a);
            if content(&view) == Content::Branch {
                let _ = self.rewrite_children(a, view, slots, batch);
            } else if let Some(&e) = self.entries.get(&a) {
                let mut cleared = e;
                cleared.brick = None;
                self.release(a, cleared, batch);
                self.set_group(a, None);
            }
        }
        // Level-1 records of changed bricks whose parent is not expanded have
        // no group to live in; everything else was rewritten above.
        if applied > 0 {
            self.write_root(source, batch);
        }
        Applied {
            applied: applied as u32,
            pending: self.pending.len() as u32,
            summarized: self.summarized - start,
        }
    }

    /// Marks the path of changed brick `k` edited.
    fn mark_edited(&mut self, k: NodeKey) {
        for level in 1..=self.shape.root_level() {
            self.edited.insert(k.ancestor(level));
        }
    }

    /// The first half of applying a changed brick: its path forgets its
    /// summaries and is marked edited, and a resident brick is uploaded
    /// again or unlinked. Returns how many cached summaries it forgot,
    /// which rewriting the path computes again.
    fn apply_brick<S: VoxelSource>(
        &mut self,
        source: &S,
        k: NodeKey,
        reupload: &mut impl FnMut(u64) -> bool,
        batch: &mut UploadBatch,
    ) -> u64 {
        self.mark_edited(k);
        let mut forgot = 0;
        for level in 1..=self.shape.root_level() {
            forgot += u64::from(self.forget_summary(k.ancestor(level)));
        }
        let Some(slot) = self.entries.get(&k).and_then(|e| e.brick) else {
            return forgot;
        };
        self.free_brick(slot, batch);
        self.set_brick(k, None);
        let view = view_at(source, k);
        if content(&view) != Content::Branch {
            return forgot;
        }
        let wanted = u64::from(slot.alloc.units) * 16;
        if reupload(wanted)
            && let Ok(new_slot) = self.place_brick(k, view, batch)
        {
            self.set_brick(k, Some(new_slot));
            self.set_layout(k, view.mask());
        }
        forgot
    }

    /// Uses the offered summaries of the replaced chunk `c`, if it has not
    /// started warming and the source holds exactly the content they were
    /// computed from: they replace whatever its old content cached, as
    /// [`warm_chunk`](Self::warm_chunk) would leave it. Returns whether they
    /// were used; an offer that does not describe the source is dropped.
    fn take_offer<S: VoxelSource>(&mut self, source: &S, c: NodeKey) -> bool {
        if self.offered.is_empty() || self.warming.contains_key(&c) {
            return false;
        }
        let Some(offer) = self.offered.remove(&c) else {
            return false;
        };
        if c.level() != WARMED_LEVEL || !offer.describes(source) {
            self.offers_stale += 1;
            return false;
        }
        self.summaries.insert_part(c, offer.into_part());
        self.warming.insert(c, 64 * 64);
        self.offers_used += 1;
        true
    }

    /// Summarizes the grandchildren of the replaced chunk `c` (and each
    /// child once its grandchildren are done) from where the last call
    /// stopped: as many as fit `budget` summaries, counted exactly before
    /// any is computed (at least one grandchild), on up to `threads`
    /// threads. The first call forgets what the chunk's old content cached.
    /// Returns whether every grandchild is done.
    fn warm_chunk<S: VoxelSource + Sync>(
        &mut self,
        source: &S,
        c: NodeKey,
        budget: u64,
        threads: usize,
    ) -> bool {
        const DONE: u32 = 64 * 64;
        let next = match self.warming.get(&c) {
            Some(&n) => n,
            None => {
                self.summaries.remove_below(&c);
                0
            }
        };
        let view = view_at(source, c);
        // Grandchildren are summarized as level-2 nodes (`summarize_level2`),
        // so only a level-4 chunk is warmed; a smaller tree's chunk (its
        // root) is summarized whole when applied.
        if c.level != 4 || content(&view) != Content::Branch {
            self.warming.insert(c, DONE);
            return true;
        }
        // The grandchildren this call summarizes, and what each costs: one
        // summary for it and one per mixed child.
        let mut units: Vec<NodeKey> = Vec::new();
        let mut planned = 0u64;
        let mut end = next;
        while end < DONE {
            if !units.is_empty() && planned >= budget {
                break;
            }
            let (s1, s2) = (end / 64, end % 64);
            let child = view.child(s1);
            if view.mask() & (1u64 << s1) == 0 || content(&child) != Content::Branch {
                end = (s1 + 1) * 64;
                continue;
            }
            if child.mask() & (1u64 << s2) != 0 {
                let grandchild = child.child(s2);
                if content(&grandchild) == Content::Branch {
                    let mixed = grandchild
                        .children()
                        .filter(|(_, b)| content(b) == Content::Branch)
                        .count();
                    planned += 1 + mixed as u64;
                    units.push(c.child(s1).child(s2));
                }
            }
            end += 1;
        }
        for (key, summary) in summarize_level2(source, &units, threads) {
            self.summarized += 1;
            self.cache_summary(key, summary);
        }
        // Children whose grandchildren are all done, from the cache.
        for s1 in next / 64..end / 64 {
            let child = view.child(s1);
            if view.mask() & (1u64 << s1) != 0 && content(&child) == Content::Branch {
                self.summary(c.child(s1), child);
            }
        }
        self.warming.insert(c, end);
        end >= DONE
    }

    /// The first half of applying a replaced chunk, once its grandchildren
    /// are summarized: it and its path forget their summaries, everything
    /// resident below it is buried (freed under a budget; its group stays,
    /// to be rewritten), and it is summarized from the source.
    fn apply_chunk<S: VoxelSource>(&mut self, source: &S, c: NodeKey) {
        self.pending_chunks.remove(&c);
        self.warming.remove(&c);
        self.offered.remove(&c);
        for level in c.level()..=self.shape.root_level() {
            self.forget_summary(c.ancestor(level));
        }
        if self.entries.get(&c).is_some_and(|e| e.group.is_some()) {
            self.bury_children(c);
        }
        let view = view_at(source, c);
        if content(&view) == Content::Branch {
            self.summary(c, view);
        }
    }

    // -------------------------------------------------------------- checks --

    /// Checks the table against the pools and the source: every record, every
    /// group and every resident block. For tests; costs a full walk.
    pub fn verify<S: VoxelSource>(
        &mut self,
        source: &S,
        nodes: &[GpuNode],
        segments: &[&[u32]],
        table: &[u32],
    ) -> Result<(), String> {
        let held = self
            .entries
            .values()
            .chain(self.graveyard.iter().map(|(_, _, e)| e));
        let (expanded, bricks) = held.fold((0, 0), |(g, b), e| {
            (
                g + usize::from(e.group.is_some()),
                b + usize::from(e.brick.is_some()),
            )
        });
        let stats = self.stats();
        if (stats.expanded, stats.bricks) != (expanded, bricks) {
            return Err(format!(
                "{} groups and {} brick ids in use, the table holds {expanded} and {bricks}",
                stats.expanded, stats.bricks
            ));
        }
        // Only the nodes on the path of a change not applied yet, and those
        // in a waiting chunk, may describe the content before it: their
        // records and blocks are checked for structure, not content.
        // Everything else, their siblings included, must match the source.
        let mut pending_paths: HashSet<NodeKey> = HashSet::default();
        for change in &self.pending {
            let k = change.key();
            for level in k.level()..=self.shape.root_level() {
                pending_paths.insert(k.ancestor(level));
            }
        }
        let affected =
            |res: &Self, key: NodeKey| pending_paths.contains(&key) || res.in_waiting_chunk(key);
        // The tree: what the root reaches through the entries' child masks.
        // Everything else is a buried entry's descendant, out of the tree
        // and freed under a budget.
        let mut reachable: HashSet<NodeKey> = HashSet::default();
        let mut stack = vec![self.shape.root()];
        while let Some(key) = stack.pop() {
            let Some(e) = self.entries.get(&key) else {
                continue;
            };
            reachable.insert(key);
            if e.group.is_some() {
                let mut kids = e.kids;
                while kids != 0 {
                    let slot = kids.trailing_zeros();
                    kids &= kids - 1;
                    stack.push(key.child(slot));
                }
            }
        }
        let mut keys = self.keys();
        keys.sort_unstable();
        for key in keys {
            if !reachable.contains(&key) {
                if !self.in_buried(key) {
                    return Err(format!("{key:?}: out of the tree but not buried"));
                }
                continue;
            }
            let e = self.entries[&key];
            let stale = affected(self, key);
            let pool = nodes.get(e.index as usize).copied();
            if pool != Some(e.record) {
                return Err(format!(
                    "{key:?}: pool holds {pool:?} at {}, table says {:?}",
                    e.index, e.record
                ));
            }
            let view = view_at(source, key);
            let expected = if key == self.shape.root() && content(&view) == Content::Air {
                Some(GpuNode::internal(0, 0, MaterialId::AIR, false))
            } else {
                self.child_record(key, view)
            };
            if !stale && expected != Some(e.record) {
                return Err(format!(
                    "{key:?}: record {:?} but the source gives {expected:?}",
                    e.record
                ));
            }
            let (mut kids, mut bricks, mut grown, mut done) = (0u64, 0u64, 0u64, 0u64);
            if e.group.is_some() {
                for slot in 0..64 {
                    if let Some(c) = self.entries.get(&key.child(slot)) {
                        kids |= 1u64 << slot;
                        if c.brick.is_some() {
                            bricks |= 1u64 << slot;
                        }
                        if c.group.is_some() {
                            grown |= 1u64 << slot;
                        }
                        if c.done() {
                            done |= 1u64 << slot;
                        }
                    }
                }
            }
            if (e.grown, e.done) != (grown, done) {
                return Err(format!(
                    "{key:?}: expanded child masks {:#x}/{:#x}, table holds {grown:#x}/{done:#x}",
                    e.grown, e.done
                ));
            }
            if (e.group.is_some() || e.brick.is_some()) && e.layout != e.record.mask() {
                return Err(format!(
                    "{key:?}: record mask {:#x}, its group or block is laid out for {:#x}",
                    e.record.mask(),
                    e.layout
                ));
            }
            if (e.kids, e.bricks) != (kids, bricks) {
                return Err(format!(
                    "{key:?}: child masks {:#x}/{:#x}, table holds {kids:#x}/{bricks:#x}",
                    e.kids, e.bricks
                ));
            }
            // Every child entry sits in the group at its rank in the layout.
            if let Some(g) = e.group {
                let mut todo = e.kids;
                while todo != 0 {
                    let slot = todo.trailing_zeros();
                    todo &= todo - 1;
                    let at = g.first + (e.layout & ((1u64 << slot) - 1)).count_ones();
                    let c = self.entries[&key.child(slot)];
                    if e.layout & (1u64 << slot) == 0 || c.index != at {
                        return Err(format!(
                            "{key:?} child {slot}: at {} outside layout {:#x} from {}",
                            c.index, e.layout, g.first
                        ));
                    }
                }
            }
            // The record points at what the entry holds.
            let linked_right = match (e.group, e.brick) {
                (Some(g), _) => e.record.kind() == Kind::Internal && e.record.reference == g.first,
                (None, Some(b)) => {
                    e.record.kind() == Kind::Brick && e.record.reference == b.alloc.block.pack()
                }
                (None, None) => key == self.shape.root() || e.record.kind() == Kind::NotResident,
            };
            if !linked_right {
                return Err(format!(
                    "{key:?}: record {:?} does not point at what the entry holds",
                    e.record
                ));
            }
            // Every record of the group, in the order of its layout.
            if let Some(g) = e.group {
                let mut todo = e.layout;
                let mut at = g.first;
                while todo != 0 {
                    let slot = todo.trailing_zeros();
                    todo &= todo - 1;
                    let have = nodes.get(at as usize).copied();
                    at += 1;
                    if affected(self, key.child(slot)) {
                        continue;
                    }
                    let want = self.child_record(key.child(slot), view.child(slot));
                    if want != have {
                        return Err(format!(
                            "{key:?} child {slot}: pool {have:?}, source {want:?}"
                        ));
                    }
                }
                // A child the layout lacks must be one a pending change made.
                let mut missing = view.mask() & !e.layout;
                while missing != 0 {
                    let slot = missing.trailing_zeros();
                    missing &= missing - 1;
                    if !affected(self, key.child(slot)) {
                        return Err(format!("{key:?} child {slot}: not in the group"));
                    }
                }
            }
            if let Some(b) = e.brick {
                let t = (b.id * 2) as usize;
                if table.get(t) != Some(&b.alloc.block.pack()) || table.get(t + 1) == Some(&0) {
                    return Err(format!("{key:?}: brick table entry {} is wrong", b.id));
                }
                if !stale {
                    let mut want = Vec::new();
                    encode_brick(
                        view,
                        b.id,
                        self.edited_bit(key),
                        &mut want,
                        &mut Tally::default(),
                    );
                    let seg = segments
                        .get(b.alloc.block.segment as usize)
                        .ok_or_else(|| format!("{key:?}: no segment {}", b.alloc.block.segment))?;
                    let at = b.alloc.block.word();
                    let have = seg.get(at..at + want.len());
                    if have != Some(&want[..]) {
                        return Err(format!(
                            "{key:?}: block at {:?} differs from the source",
                            b.alloc.block
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

/// A cheap summary for a replaced chunk that is not applied yet: the
/// material found by following the lowest non-air child down, and half the
/// volume of its non-air children. Deterministic, and replaced by the exact
/// summary when the chunk is applied.
fn stand_in<N: NodeView>(key: NodeKey, view: N) -> Summary {
    let mut node = view;
    let material = loop {
        match content(&node) {
            Content::Uniform(m) => break m,
            Content::Leaf => {
                break node
                    .palette()
                    .as_slice()
                    .first()
                    .copied()
                    .unwrap_or(MaterialId::AIR);
            }
            Content::Branch => node = node.child(node.mask().trailing_zeros()),
            Content::Air | Content::Unloaded => break MaterialId::AIR,
        }
    };
    let child = node_volume(key.level().saturating_sub(1));
    Summary {
        material,
        solid: u64::from(view.mask().count_ones()) * child / 2,
    }
}

/// The summaries of the level-2 nodes `keys` (only level 2: a uniform
/// child weighs a level-1 node's volume) and of their mixed children,
/// computed in up to `threads` tasks: for each node, its mixed
/// children's then its own, as [`Residency::summary`] computes them with
/// nothing cached, and as [`ChunkSummaries`] computes them where a chunk is
/// built (the same function). Pure, so the result does not depend on the
/// threads.
fn summarize_level2<S: VoxelSource + Sync>(
    source: &S,
    keys: &[NodeKey],
    threads: usize,
) -> Vec<(NodeKey, Summary)> {
    let run = |chunk: &[NodeKey]| {
        let mut out = Vec::new();
        let mut tally = Tally::default();
        for &key in chunk {
            summarize_level2_node(view_at(source, key), key, &mut tally, &mut |k, s| {
                out.push((k, s));
            });
        }
        out
    };
    // A few nodes are not worth a task.
    let tasks = if keys.len() < 8 { 1 } else { threads };
    in_parallel(keys, tasks, run)
}

/// The brick cache's own task pool: up to 8 threads, made once. Not Bevy's
/// compute task pool: a scope there, from inside the render system, made
/// frames wait on Bevy's own tasks (walking, p95 and p99 5 to 7 ms higher).
fn pool() -> &'static TaskPool {
    static POOL: std::sync::OnceLock<TaskPool> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get().clamp(1, 8));
        TaskPoolBuilder::new()
            .num_threads(threads)
            .thread_name("brick cache".to_string())
            .build()
    })
}

/// Threads of the brick cache's task pool, for sizing parallel work.
pub fn pool_threads() -> usize {
    pool().thread_num().max(1)
}

/// `run` over `items` split into up to `tasks` contiguous parts, on the
/// brick cache's task pool (no threads are spawned per call), the results
/// concatenated in order. A panic in a part is raised again here.
fn in_parallel<T: Sync, R: Send + 'static>(
    items: &[T],
    tasks: usize,
    run: impl Fn(&[T]) -> Vec<R> + Sync,
) -> Vec<R> {
    if tasks <= 1 || items.len() <= 1 {
        return run(items);
    }
    let per = items.len().div_ceil(tasks);
    let run = &run;
    let parts = pool().scope(|scope| {
        for part in items.chunks(per) {
            scope.spawn(async move {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(part)))
            });
        }
    });
    let mut out = Vec::with_capacity(items.len());
    for part in parts {
        match part {
            Ok(results) => out.extend(results),
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }
    out
}

/// Encodes `jobs` (key, brick id, edited) in up to `threads` tasks,
/// each into its own buffer, and returns the blocks in job order.
fn encode_parallel<S: VoxelSource + Sync>(
    source: &S,
    jobs: &[(NodeKey, u32, bool)],
    threads: usize,
) -> Vec<Option<(Vec<u32>, crate::brick_cache::format::brick::EncodedBrick)>> {
    let encode = |chunk: &[(NodeKey, u32, bool)]| {
        let mut tally = Tally::default();
        chunk
            .iter()
            .map(|&(key, id, edited)| {
                let view = view_at(source, key);
                (content(&view) == Content::Branch).then(|| {
                    let mut words = Vec::new();
                    let encoded = encode_brick(view, id, edited, &mut words, &mut tally);
                    (words, encoded)
                })
            })
            .collect::<Vec<_>>()
    };
    // Small batches are not worth a task.
    let tasks = if jobs.len() < 32 { 1 } else { threads };
    in_parallel(jobs, tasks, encode)
}

#[cfg(test)]
mod tests {
    use super::in_parallel;
    use super::*;
    use world_core::tree::CHUNK_LEVEL;
    use world_core::{Edit, VoxelPos, VoxelTree};
    use world_gen::{ChunkCoord, CoarseSource, Generator, GeneratorVersion, TerrainConfig};

    fn layout() -> PoolLayout {
        PoolLayout {
            node_count: 1 << 20,
            segments: vec![256 << 20],
            brick_ids: 1 << 18,
            allocs_per_segment: 1 << 18,
            node_groups: 1 << 18,
            upload_budget: 1 << 30,
            request_capacity: 1024,
            victim_capacity: 1024,
        }
    }

    /// Real terrain, as the residency workers build it: the default
    /// terrain's surface chunk in a central column and the chunk below it,
    /// both at full detail, and the surface chunk of the next column built
    /// from coarse cells, each built in a tree of its own and moved into
    /// one world tree. Returns the tree and the chunks' lowest voxels.
    fn terrain() -> (VoxelTree, Vec<VoxelPos>) {
        let generator = Generator::new(0x5EED, GeneratorVersion::CURRENT, TerrainConfig::default())
            .expect("the default config is valid");
        let surface = |cx: u32| {
            let probe = ChunkCoord::new(cx, 0, 512).unwrap().min_voxel();
            let (x, z) = (probe.x + 512, probe.z + 512);
            let height = generator.column(x, z).unwrap().height;
            ChunkCoord::containing(VoxelPos::new(x, height, z)).unwrap()
        };
        let top = surface(512);
        let below = ChunkCoord::new(top.x(), top.y() - 1, top.z()).unwrap();
        let far = surface(513);
        let mut world = VoxelTree::unloaded();
        let mut mins = Vec::new();
        for (coord, coarse) in [(top, false), (below, false), (far, true)] {
            let min = coord.min_voxel();
            let mut t = VoxelTree::new();
            if coarse {
                let columns = generator.coarse_columns(coord, 2).unwrap();
                t.build_region(CHUNK_LEVEL, min, &CoarseSource::new(columns))
                    .unwrap();
            } else {
                t.build_region(CHUNK_LEVEL, min, &generator.chunk_source(coord))
                    .unwrap();
            }
            world.insert_chunk(min, t.take_chunk(min).unwrap()).unwrap();
            mins.push(min);
        }
        (world, mins)
    }

    fn chunk_key(res: &Residency, min: VoxelPos) -> NodeKey {
        NodeKey::containing(res.shape().local(min).unwrap(), CHUNK_LEVEL)
    }

    /// A table over `snap` with every chunk of `mins` applied as replaced,
    /// offered `offers` first; the upload of applying them, and the
    /// summaries applying them computed.
    fn apply_chunks(
        snap: &world_core::TreeSnapshot,
        mins: &[VoxelPos],
        offers: Vec<ChunkSummaries>,
    ) -> (Residency, UploadBatch, u64) {
        let shape = Shape::of(snap);
        let mut res = Residency::new(shape, &layout()).unwrap();
        let mut batch = UploadBatch::default();
        res.reset(snap, &mut batch);
        // Expand the paths down to the chunks, as stage 1 keeps them.
        for &min in mins {
            let key = chunk_key(&res, min);
            for level in (CHUNK_LEVEL..=shape.root_level()).rev() {
                let _ = res.expand(snap, key.ancestor(level), &mut batch);
            }
        }
        batch.clear();
        res.offer_summaries(offers);
        let changes: Vec<_> = mins
            .iter()
            .map(|&m| Change::Chunk(chunk_key(&res, m)))
            .collect();
        res.queue_changes(changes);
        let applied = res.apply_changes(snap, None, u64::MAX, 4, |_| true, &mut batch);
        assert_eq!(applied.pending, 0);
        (res, batch, applied.summarized)
    }

    fn summaries_below(res: &Residency, c: NodeKey) -> Vec<(NodeKey, Summary)> {
        let mut v: Vec<_> = res
            .summaries
            .iter()
            .filter(|(k, _)| k.level() < c.level() && k.ancestor(c.level()) == c)
            .map(|(k, s)| (k, *s))
            .collect();
        v.sort_unstable_by_key(|(k, _)| *k);
        v
    }

    fn all_summaries(res: &Residency) -> Vec<(NodeKey, Summary)> {
        let mut v: Vec<_> = res.summaries.iter().map(|(k, s)| (k, *s)).collect();
        v.sort_unstable_by_key(|(k, _)| *k);
        v
    }

    /// Summaries computed where a chunk is built are exactly the ones the
    /// render thread computes on applying it, node for node, on real
    /// terrain chunks at full and coarse detail; and a table given them
    /// writes exactly the same GPU records while computing none of them.
    #[test]
    fn worker_summaries_equal_the_render_threads_on_real_terrain() {
        let (world, mins) = terrain();
        let snap = world.snapshot();
        let offers: Vec<_> = mins
            .iter()
            .map(|&m| ChunkSummaries::compute(m, &world.chunk_data(m).unwrap()).unwrap())
            .collect();
        let (computed, computed_batch, a) = apply_chunks(&snap, &mins, Vec::new());
        for offer in &offers {
            let c = chunk_key(&computed, offer.min());
            let expected = summaries_below(&computed, c);
            assert!(expected.len() > 100, "{} mixed nodes", expected.len());
            assert_eq!(
                offer.len(),
                expected.len(),
                "every mixed node below the chunk"
            );
            for (k, s) in expected {
                assert_eq!(offer.get(&k), Some(s), "{k:?}");
            }
        }
        let (given, given_batch, b) = apply_chunks(&snap, &mins, offers);
        assert_eq!(given.offers_used(), (3, 0));
        assert_eq!(all_summaries(&given), all_summaries(&computed));
        let records = |b: &UploadBatch| b.node_writes().collect::<Vec<_>>();
        assert!(!records(&computed_batch).is_empty());
        assert_eq!(records(&given_batch), records(&computed_batch));
        assert!(
            b * 20 < a,
            "applying computed {b} summaries with offers, {a} without"
        );
    }

    /// An offer is used only while the source holds the very content it
    /// was computed from: a chunk edited after its summaries were computed
    /// (an edit applied on joining, or one in the same frame) and a chunk
    /// replaced again are summarized on the render thread instead, with the
    /// same result as never offering them.
    #[test]
    fn an_edited_chunk_never_takes_summaries_of_its_old_content() {
        let (mut world, mins) = terrain();
        let offers: Vec<_> = mins
            .iter()
            .map(|&m| ChunkSummaries::compute(m, &world.chunk_data(m).unwrap()).unwrap())
            .collect();
        // Edit the first chunk; replace the second with an equal copy built
        // again (same content, other arenas); leave the third.
        let p = VoxelPos::new(mins[0].x + 300, mins[0].y + 500, mins[0].z + 300);
        let flipped = if world.get(p).is_air() {
            MaterialId(3)
        } else {
            MaterialId::AIR
        };
        world.apply(&[Edit::new(p, flipped)]).unwrap();
        let mut copy = world.extract(CHUNK_LEVEL, mins[1]).unwrap();
        world
            .insert_chunk(mins[1], copy.take_chunk(mins[1]).unwrap())
            .unwrap();
        let snap = world.snapshot();
        assert!(!offers[0].describes(&snap) && !offers[1].describes(&snap));
        assert!(offers[2].describes(&snap));
        let (computed, computed_batch, _) = apply_chunks(&snap, &mins, Vec::new());
        let (given, given_batch, _) = apply_chunks(&snap, &mins, offers);
        assert_eq!(given.offers_used(), (1, 2));
        assert_eq!(all_summaries(&given), all_summaries(&computed));
        let records = |b: &UploadBatch| b.node_writes().collect::<Vec<_>>();
        assert_eq!(records(&given_batch), records(&computed_batch));
    }

    /// Offers for chunks that are not waiting are not kept, and sources
    /// that do not store chunks (a dense grid) never use one.
    #[test]
    fn offers_are_kept_only_for_waiting_chunks() {
        let (world, mins) = terrain();
        let snap = world.snapshot();
        let mut res = Residency::new(Shape::of(&snap), &layout()).unwrap();
        let mut batch = UploadBatch::default();
        res.reset(&snap, &mut batch);
        let offer = ChunkSummaries::compute(mins[0], &world.chunk_data(mins[0]).unwrap()).unwrap();
        res.offer_summaries([offer]);
        assert_eq!(res.offers_held(), 1);
        res.apply_changes(&snap, None, u64::MAX, 1, |_| true, &mut batch);
        assert_eq!(res.offers_held(), 0, "no change waits for it");
        let air = ChunkSummaries::compute(
            VoxelPos::new(0, 0, 0),
            &VoxelTree::new().chunk_data(VoxelPos::new(0, 0, 0)).unwrap(),
        );
        assert!(air.is_none(), "air has nothing to summarize");
        let grid = world_core::DenseGrid::new(3, VoxelPos::new(0, 0, 0));
        let offer = ChunkSummaries::compute(mins[0], &world.chunk_data(mins[0]).unwrap()).unwrap();
        assert!(!offer.describes(&grid));
    }

    #[test]
    fn parallel_parts_keep_their_order() {
        let items: Vec<u32> = (0..1000).collect();
        let doubled = in_parallel(&items, 7, |part| part.iter().map(|x| x * 2).collect());
        assert_eq!(doubled, items.iter().map(|x| x * 2).collect::<Vec<_>>());
    }

    #[test]
    #[should_panic(expected = "a part failed")]
    fn a_panic_in_a_part_is_raised_again() {
        let items: Vec<u32> = (0..1000).collect();
        in_parallel(&items, 4, |part| {
            assert!(!part.contains(&700), "a part failed");
            part.to_vec()
        });
    }
}
