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
use world_core::MaterialId;
use world_core::tree::{NodeView, VoxelSource};

use super::batch::UploadBatch;
use super::key::{NodeKey, Shape, view_at};
use super::select::GrowthState;
use crate::brick_cache::format::brick::encode_brick;
use crate::brick_cache::format::summary::{
    Content, Summary, Tally, content, count_voxels, exact_summary, node_volume,
};
use crate::brick_cache::format::{BrickRef, GpuNode, ROOT_NODE};
use crate::brick_cache::pool::alloc::{BrickAlloc, BrickSpace, IdSpace, NodeAlloc, NodeSpace};
use crate::brick_cache::pool::layout::PoolLayout;

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
}

impl Entry {
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
    entries: HashMap<NodeKey, Entry>,
    /// Cached summaries of nodes at and above the chunk level.
    summaries: HashMap<NodeKey, Summary>,
    /// Cached summaries below the chunk level, by chunk, so a replaced
    /// chunk forgets its own at once.
    chunk_summaries: HashMap<NodeKey, HashMap<NodeKey, Summary>>,
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
}

impl Residency {
    /// An empty table for a tree of `shape` in pools of `layout`. The first
    /// [`reset`](Self::reset) writes the root.
    pub fn new(shape: Shape, layout: &PoolLayout) -> Option<Self> {
        Some(Self {
            shape,
            // Growing a table of a million entries rehashes it in one go,
            // over 100 ms on the floor Mac: reserve what a full node pool
            // typically holds (entries are the mixed records, about a third
            // of them) so that never happens mid-walk.
            entries: HashMap::with_capacity_and_hasher(
                (layout.node_count as usize).saturating_mul(7) / 16,
                Default::default(),
            ),
            summaries: HashMap::default(),
            chunk_summaries: HashMap::default(),
            edited: HashSet::default(),
            pending: Vec::new(),
            pending_chunks: HashSet::default(),
            warming: HashMap::default(),
            summarized: 0,
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

    /// Resident bricks: each holds exactly one brick id. O(1), unlike
    /// [`stats`](Self::stats), which scans the table.
    pub fn brick_count(&self) -> usize {
        self.ids.in_use() as usize
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
            .map(|(&k, _)| k)
            .collect();
        v.sort_unstable();
        v
    }

    /// Every node with a record, sorted.
    pub fn keys(&self) -> Vec<NodeKey> {
        let mut v: Vec<_> = self.entries.keys().copied().collect();
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
        self.edited.contains(&key)
    }

    /// The level of the chunks: summaries below it are kept per chunk.
    pub fn chunk_level(&self) -> u32 {
        world_core::tree::CHUNK_LEVEL.min(self.shape.root_level())
    }

    fn cached_summary(&self, key: NodeKey) -> Option<Summary> {
        let split = self.chunk_level();
        if key.level() >= split {
            self.summaries.get(&key).copied()
        } else {
            self.chunk_summaries
                .get(&key.ancestor(split))?
                .get(&key)
                .copied()
        }
    }

    fn cache_summary(&mut self, key: NodeKey, s: Summary) {
        let split = self.chunk_level();
        if key.level() >= split {
            self.summaries.insert(key, s);
        } else {
            self.chunk_summaries
                .entry(key.ancestor(split))
                .or_default()
                .insert(key, s);
        }
    }

    fn forget_summary(&mut self, key: NodeKey) {
        let split = self.chunk_level();
        if key.level() >= split {
            self.summaries.remove(&key);
        } else if let Some(m) = self.chunk_summaries.get_mut(&key.ancestor(split)) {
            m.remove(&key);
        }
    }

    /// Node summaries computed since the table was made: a count of work,
    /// for budgets and tests.
    pub fn summaries_computed(&self) -> u64 {
        self.summarized
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
        let edited = self.edited_bit(key);
        match content(&view) {
            Content::Air => None,
            Content::Uniform(m) => Some(GpuNode::uniform(m, edited)),
            Content::Unloaded => Some(GpuNode::not_resident(0, MaterialId::AIR, edited)),
            Content::Branch | Content::Leaf => {
                let material = self.summary(key, view).material;
                let mask = view.mask();
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
        self.entries
            .entry(key)
            .and_modify(|e| {
                e.index = index;
                e.record = record;
            })
            .or_insert_with(|| Entry::new(index, record));
        if key != self.shape.root()
            && let Some(p) = self.entries.get_mut(&key.parent())
        {
            p.kids |= 1u64 << key.slot_in_parent();
        }
    }

    /// Removes the entry of `key`, keeping its parent's child masks.
    fn remove_entry(&mut self, key: NodeKey) -> Option<Entry> {
        let e = self.entries.remove(&key)?;
        if key != self.shape.root()
            && let Some(p) = self.entries.get_mut(&key.parent())
        {
            let bit = 1u64 << key.slot_in_parent();
            p.kids &= !bit;
            p.bricks &= !bit;
        }
        Some(e)
    }

    /// Sets the resident brick of level-1 node `key`, keeping its parent's
    /// brick mask.
    fn set_brick(&mut self, key: NodeKey, slot: Option<BrickSlot>) {
        let Some(e) = self.entries.get_mut(&key) else {
            return;
        };
        e.brick = slot;
        if key != self.shape.root()
            && let Some(p) = self.entries.get_mut(&key.parent())
        {
            let bit = 1u64 << key.slot_in_parent();
            if slot.is_some() {
                p.bricks |= bit;
            } else {
                p.bricks &= !bit;
            }
        }
    }

    /// Records the mask the group or brick block of `key` was laid out for.
    fn set_layout(&mut self, key: NodeKey, mask: u64) {
        if let Some(e) = self.entries.get_mut(&key) {
            e.layout = mask;
        }
    }

    /// Before the record of `key` is written again for content whose mask
    /// is `mask`: drops what the entry holds below it (children, group,
    /// brick) when that was laid out for another mask, or when it lies in a
    /// replaced chunk that waits (it describes the old content, and applying
    /// the chunk would drop it anyway). The record then points at nothing
    /// it cannot read; stage 1 refines the node again.
    fn unlink_stale(&mut self, key: NodeKey, mask: u64, batch: &mut UploadBatch) {
        let Some(&e) = self.entries.get(&key) else {
            return;
        };
        if e.group.is_none() && e.brick.is_none() {
            return;
        }
        if e.layout == mask && !self.in_waiting_chunk(key) {
            return;
        }
        self.release(key, e, batch);
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.group = None;
        }
        self.set_brick(key, None);
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
        // Children that became air or uniform lose their entries.
        for slot in 0..64 {
            let ck = key.child(slot);
            if !self.entries.contains_key(&ck) {
                continue;
            }
            let gone = mask & (1u64 << slot) == 0 || content(&view.child(slot)) != Content::Branch;
            if gone {
                self.drop_subtree(ck, batch);
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
                        if let Some(e) = self.entries.get_mut(&key) {
                            e.group = None;
                        }
                        let children: Vec<u32> = (0..64).collect();
                        for slot in children {
                            self.drop_subtree(key.child(slot), batch);
                        }
                        return Err(Refused::PoolFull);
                    }
                }
            }
        };
        if let Some(e) = self.entries.get_mut(&key) {
            e.group = Some(group);
            e.layout = mask;
        }

        for (k, (slot, child)) in view.children().enumerate() {
            let ck = key.child(slot);
            let index = group.first + k as u32;
            self.unlink_stale(ck, child.mask(), batch);
            let Some(record) = self.child_record(ck, child) else {
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
            self.unlink_stale(ck, child.mask(), batch);
            if let Some(record) = self.child_record(ck, child) {
                if branch {
                    self.put_entry(ck, index, record);
                }
                batch.write_node(index, record);
            }
        }
        Ok(())
    }

    /// Rewrites the root record from the source.
    fn write_root<S: VoxelSource>(&mut self, source: &S, batch: &mut UploadBatch) {
        let root = self.shape.root();
        let view = source.root();
        let record = match content(&view) {
            Content::Air => GpuNode::internal(0, 0, MaterialId::AIR, false),
            _ => self.child_record(root, view).unwrap_or(GpuNode::internal(
                0,
                0,
                MaterialId::AIR,
                false,
            )),
        };
        self.entries
            .entry(root)
            .and_modify(|e| e.record = record)
            .or_insert_with(|| Entry::new(ROOT_NODE, record));
        batch.write_node(ROOT_NODE, record);
    }

    /// Drops everything and writes the root from `source`, not expanded.
    pub fn reset<S: VoxelSource>(&mut self, source: &S, batch: &mut UploadBatch) {
        let root = self.shape.root();
        if let Some(e) = self.entries.remove(&root) {
            self.release(root, e, batch);
        }
        self.entries.clear();
        self.summaries.clear();
        self.chunk_summaries.clear();
        self.pending.clear();
        self.pending_chunks.clear();
        self.warming.clear();
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
        if self.in_waiting_chunk(key) {
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
        let mut cleared = e;
        cleared.brick = None;
        self.release(key, cleared, batch);
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.group = None;
        }
        self.write_record(key, e.record.unlinked(), batch);
        Ok(())
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
        if self.in_waiting_chunk(key) {
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
                && !self.in_waiting_chunk(key);
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
            (Some(key), Some(current)) if current.pack() == block => {
                self.evict_brick(key, batch).is_ok()
            }
            _ => false,
        }
    }

    /// Applies an edit batch's changed bricks at once (see
    /// [`apply_changes`](Self::apply_changes)).
    pub fn apply_edits<S: VoxelSource>(
        &mut self,
        source: &S,
        dirty: &[NodeKey],
        reupload: impl FnMut(u64) -> bool,
        batch: &mut UploadBatch,
    ) {
        self.queue_changes(dirty.iter().map(|&k| Change::Brick(k)));
        self.apply_changes(source, None, u64::MAX, reupload, batch);
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
    /// one), until they have computed `max_summaries` node summaries. A
    /// replaced chunk is summarized first, a grandchild at a time and across
    /// calls if the budget runs out (at least one grandchild per call), and
    /// applied once that is done. The rest wait for the next call, and until
    /// then the records above them describe the content they had before.
    ///
    /// A changed brick: `reupload` decides, in that order, which resident
    /// changed bricks are uploaded again (it gets the block bytes the brick
    /// needs and returns whether it fits the budget); the rest become not
    /// resident. A replaced chunk drops everything resident below it (it
    /// described the old content) and is summarized afresh. Then every
    /// record on the applied changes' paths is rewritten from the source.
    pub fn apply_changes<S: VoxelSource>(
        &mut self,
        source: &S,
        eye: Option<[f32; 3]>,
        max_summaries: u64,
        mut reupload: impl FnMut(u64) -> bool,
        batch: &mut UploadBatch,
    ) -> Applied {
        let start = self.summarized;
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
        for change in order {
            let spent = self.summarized - start;
            if worked && spent >= max_summaries {
                rest.push(change);
                continue;
            }
            worked = true;
            let done = match change {
                Change::Brick(k) => {
                    self.apply_brick(source, k, &mut reupload, batch);
                    true
                }
                Change::Chunk(c) => {
                    let ready = self.warm_chunk(source, c, max_summaries.saturating_sub(spent));
                    if ready {
                        self.apply_chunk(source, c, batch);
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
                if let Some(entry) = self.entries.get_mut(&a) {
                    entry.group = None;
                }
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

    /// The first half of applying a changed brick: its path forgets its
    /// summaries and is marked edited, and a resident brick is uploaded
    /// again or unlinked.
    fn apply_brick<S: VoxelSource>(
        &mut self,
        source: &S,
        k: NodeKey,
        reupload: &mut impl FnMut(u64) -> bool,
        batch: &mut UploadBatch,
    ) {
        for level in 1..=self.shape.root_level() {
            let a = k.ancestor(level);
            self.forget_summary(a);
            self.edited.insert(a);
        }
        let Some(slot) = self.entries.get(&k).and_then(|e| e.brick) else {
            return;
        };
        self.free_brick(slot, batch);
        self.set_brick(k, None);
        let view = view_at(source, k);
        if content(&view) != Content::Branch {
            return;
        }
        let wanted = u64::from(slot.alloc.units) * 16;
        if reupload(wanted)
            && let Ok(new_slot) = self.place_brick(k, view, batch)
        {
            self.set_brick(k, Some(new_slot));
            self.set_layout(k, view.mask());
        }
    }

    /// Summarizes the grandchildren of the replaced chunk `c` (and each
    /// child once its grandchildren are done) from where the last call
    /// stopped, until `budget` summaries were computed (at least one
    /// grandchild). The first call forgets what the chunk's old content
    /// cached. Returns whether every grandchild is done.
    fn warm_chunk<S: VoxelSource>(&mut self, source: &S, c: NodeKey, budget: u64) -> bool {
        const DONE: u32 = 64 * 64;
        let next = match self.warming.get(&c) {
            Some(&n) => n,
            None => {
                self.chunk_summaries.remove(&c);
                0
            }
        };
        let view = view_at(source, c);
        if c.level < 3 || content(&view) != Content::Branch {
            self.warming.insert(c, DONE);
            return true;
        }
        let start = self.summarized;
        let mut next = next;
        let mut progressed = false;
        while next < DONE {
            if progressed && self.summarized - start >= budget {
                break;
            }
            let (s1, s2) = (next / 64, next % 64);
            let child = view.child(s1);
            if view.mask() & (1u64 << s1) == 0 || content(&child) != Content::Branch {
                next = (s1 + 1) * 64;
                continue;
            }
            if child.mask() & (1u64 << s2) != 0 {
                let grandchild = child.child(s2);
                if content(&grandchild) == Content::Branch {
                    self.summary(c.child(s1).child(s2), grandchild);
                    progressed = true;
                }
            }
            next += 1;
            if next % 64 == 0 {
                // From the cached grandchildren.
                self.summary(c.child(s1), child);
            }
        }
        self.warming.insert(c, next);
        next >= DONE
    }

    /// The first half of applying a replaced chunk, once its grandchildren
    /// are summarized: it and its path forget their summaries, everything
    /// resident below it is dropped (its group stays, to be rewritten), and
    /// it is summarized from the source.
    fn apply_chunk<S: VoxelSource>(&mut self, source: &S, c: NodeKey, batch: &mut UploadBatch) {
        self.pending_chunks.remove(&c);
        self.warming.remove(&c);
        for level in c.level()..=self.shape.root_level() {
            self.forget_summary(c.ancestor(level));
        }
        if let Some(e) = self.entries.get(&c)
            && e.group.is_some()
        {
            let kids = e.kids;
            for slot in 0..64 {
                if kids & (1u64 << slot) != 0 {
                    self.drop_subtree(c.child(slot), batch);
                }
            }
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
        let expanded = self.entries.values().filter(|e| e.group.is_some()).count();
        let bricks = self.entries.values().filter(|e| e.brick.is_some()).count();
        let stats = self.stats();
        if (stats.expanded, stats.bricks) != (expanded, bricks) {
            return Err(format!(
                "{} groups and {} brick ids in use, the table holds {expanded} and {bricks}",
                stats.expanded, stats.bricks
            ));
        }
        let mut keys = self.keys();
        keys.sort_unstable();
        for key in keys {
            let e = self.entries[&key];
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
            if expected != Some(e.record) {
                return Err(format!(
                    "{key:?}: record {:?} but the source gives {expected:?}",
                    e.record
                ));
            }
            let (mut kids, mut bricks) = (0u64, 0u64);
            if e.group.is_some() {
                for slot in 0..64 {
                    if let Some(c) = self.entries.get(&key.child(slot)) {
                        kids |= 1u64 << slot;
                        if c.brick.is_some() {
                            bricks |= 1u64 << slot;
                        }
                    }
                }
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
            if let Some(g) = e.group {
                for (k, (slot, child)) in view.children().enumerate() {
                    let want = self.child_record(key.child(slot), child);
                    let have = nodes.get((g.first + k as u32) as usize).copied();
                    if want != have {
                        return Err(format!(
                            "{key:?} child {slot}: pool {have:?}, source {want:?}"
                        ));
                    }
                }
            }
            if let Some(b) = e.brick {
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
                let t = (b.id * 2) as usize;
                if table.get(t) != Some(&b.alloc.block.pack()) || table.get(t + 1) == Some(&0) {
                    return Err(format!("{key:?}: brick table entry {} is wrong", b.id));
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

/// Encodes `jobs` (key, brick id, edited) on up to `threads` scoped threads,
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
    // Small batches are not worth a thread.
    if threads <= 1 || jobs.len() < 32 {
        return encode(jobs);
    }
    let per = jobs.len().div_ceil(threads);
    std::thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .chunks(per)
            .map(|chunk| scope.spawn(move || encode(chunk)))
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    })
}
