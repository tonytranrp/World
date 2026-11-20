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
//! - Masks and materials always describe the current source: an edit
//!   rewrites the records on its path in the same batch, and a changed brick
//!   that cannot be uploaded is made not resident, never left stale.

use bevy::platform::collections::{HashMap, HashSet};
use world_core::MaterialId;
use world_core::tree::{NodeView, VoxelSource};

use super::batch::UploadBatch;
use super::key::{NodeKey, Shape, view_at};
use super::select::GrowthState;
use crate::brick_cache::format::brick::encode_brick;
use crate::brick_cache::format::summary::{
    Content, Summary, Tally, content, exact_summary, node_volume,
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
}

impl Entry {
    fn new(index: u32, record: GpuNode) -> Self {
        Self {
            index,
            record,
            group: None,
            brick: None,
            last_requested: 0,
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
    summaries: HashMap<NodeKey, Summary>,
    edited: HashSet<NodeKey>,
    nodes: NodeSpace,
    bricks: BrickSpace,
    ids: IdSpace,
    /// The level-1 node each brick id holds.
    id_keys: Vec<Option<NodeKey>>,
    frame: u32,
    tally: Tally,
    /// Stage 1's walk in progress: nodes still to visit (see
    /// [`select_budgeted`](super::select::select_budgeted)).
    walk: Vec<NodeKey>,
    /// Stage 1's growth pass in progress and carried candidates.
    growth: GrowthState,
    /// Stage 1's detail threshold multiplier under node pool pressure (see
    /// `frame::adapt_pressure`); 1 when the pool has room.
    pressure: f32,
    /// The next shrink walk runs whole: the pressure just changed, and its
    /// collapses should land before it is judged again.
    full_shrink: bool,
    /// The frame the pressure last rose.
    pressure_raised_at: u32,
}

/// What stage 1 needs of one node, from a single table lookup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeState {
    pub expanded: bool,
    pub brick_resident: bool,
    /// Children with content, by slot.
    pub mask: u64,
}

impl Residency {
    /// An empty table for a tree of `shape` in pools of `layout`. The first
    /// [`reset`](Self::reset) writes the root.
    pub fn new(shape: Shape, layout: &PoolLayout) -> Option<Self> {
        Some(Self {
            shape,
            entries: HashMap::default(),
            summaries: HashMap::default(),
            edited: HashSet::default(),
            nodes: NodeSpace::new(layout.node_count, layout.node_groups)?,
            bricks: BrickSpace::new(&layout.segments, layout.allocs_per_segment),
            ids: IdSpace::new(layout.brick_ids),
            id_keys: vec![None; layout.brick_ids as usize],
            frame: 1,
            tally: Tally::default(),
            walk: Vec::new(),
            growth: GrowthState::default(),
            pressure: 1.0,
            full_shrink: false,
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

    pub fn stats(&self) -> ResidencyStats {
        ResidencyStats {
            entries: self.entries.len(),
            expanded: self.entries.values().filter(|e| e.group.is_some()).count(),
            bricks: self.entries.values().filter(|e| e.brick.is_some()).count(),
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

    /// Sets the pressure; a change makes the next shrink walk run whole.
    pub fn set_pressure(&mut self, pressure: f32) {
        if pressure != self.pressure {
            self.full_shrink = true;
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

    /// Whether the next shrink walk should run whole; clears the request.
    pub fn take_full_shrink(&mut self) -> bool {
        std::mem::take(&mut self.full_shrink)
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

    /// The summary of the node `key`, whose view is `view`; cached for
    /// branches until an edit below them invalidates it.
    fn summary<N: NodeView>(&mut self, key: NodeKey, view: N) -> Summary {
        if let Some(s) = self.summaries.get(&key) {
            return *s;
        }
        let kind = content(&view);
        let s = match kind {
            Content::Air | Content::Unloaded => Summary::default(),
            Content::Uniform(m) => Summary {
                material: m,
                solid: node_volume(key.level()),
            },
            Content::Leaf => exact_summary(view, 0),
            Content::Branch if key.level == 1 => exact_summary(view, 1),
            Content::Branch => {
                let mut tally = Tally::default();
                for (slot, child) in view.children() {
                    let c = self.summary(key.child(slot), child);
                    tally.add(c.material, c.solid);
                }
                tally.summary()
            }
        };
        if kind == Content::Branch {
            self.summaries.insert(key, s);
        }
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

    /// Forgets everything below and including `key` without writing records:
    /// the caller rewrites the group or record that referred to it.
    fn drop_subtree(&mut self, key: NodeKey, batch: &mut UploadBatch) {
        let Some(e) = self.entries.remove(&key) else {
            return;
        };
        self.release(key, e, batch);
    }

    /// Frees what an entry holds: its children (recursively), group and brick.
    fn release(&mut self, key: NodeKey, e: Entry, batch: &mut UploadBatch) {
        if let Some(g) = e.group {
            let mask = e.record.mask();
            for slot in 0..64 {
                if mask & (1u64 << slot) != 0 {
                    self.drop_subtree(key.child(slot), batch);
                }
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
        }

        for (k, (slot, child)) in view.children().enumerate() {
            let ck = key.child(slot);
            let index = group.first + k as u32;
            let Some(record) = self.child_record(ck, child) else {
                continue;
            };
            if content(&child) == Content::Branch {
                self.entries
                    .entry(ck)
                    .and_modify(|e| {
                        e.index = index;
                        e.record = record;
                    })
                    .or_insert_with(|| Entry::new(index, record));
            }
            batch.write_node(index, record);
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
        let view = view_at(source, key);
        if content(&view) != Content::Branch {
            return Err(Refused::NotApplicable);
        }
        let slot = self.place_brick(key, view, batch)?;
        let edited = self.edited_bit(key);
        let material = self.summary(key, view).material;
        let record = GpuNode::brick(view.mask(), slot.alloc.block, material, edited);
        if let Some(e) = self.entries.get_mut(&key) {
            e.brick = Some(slot);
        }
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
            let ok = key.level == 1 && self.entries.get(&key).is_some_and(|e| e.brick.is_none());
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
            self.summaries.insert(*key, encoded.summary);
            batch.scatter(start as u32, alloc.block, encoded.words);
            batch.write_table(*id, Some(alloc.block), self.frame);
            if let Some(k) = self.id_keys.get_mut(*id as usize) {
                *k = Some(*key);
            }
            let slot = BrickSlot { id: *id, alloc };
            let record =
                GpuNode::brick(encoded.mask, alloc.block, encoded.summary.material, *edited);
            if let Some(e) = self.entries.get_mut(key) {
                e.brick = Some(slot);
            }
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
        self.summaries.insert(key, encoded.summary);
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
        let Some(e) = self.entries.get_mut(&key) else {
            return Err(Refused::NotApplicable);
        };
        let Some(slot) = e.brick.take() else {
            return Err(Refused::AlreadyDone);
        };
        let record = e.record.unlinked();
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

    /// Applies an edit batch's changed bricks. `reupload` decides, in the
    /// caller's priority order, which resident changed bricks are uploaded
    /// again this frame (it gets the block bytes the brick needs and returns
    /// whether it fits the budget); the rest become not resident. Every
    /// record on the changed paths is rewritten from the source.
    pub fn apply_edits<S: VoxelSource>(
        &mut self,
        source: &S,
        dirty: &[NodeKey],
        mut reupload: impl FnMut(u64) -> bool,
        batch: &mut UploadBatch,
    ) {
        let root_level = self.shape.root_level();
        for &k in dirty {
            for level in 1..=root_level {
                let a = k.ancestor(level);
                self.summaries.remove(&a);
                self.edited.insert(a);
            }
        }

        // Resident changed bricks: upload again within budget, else unlink.
        for &k in dirty {
            let Some(slot) = self.entries.get(&k).and_then(|e| e.brick) else {
                continue;
            };
            self.free_brick(slot, batch);
            if let Some(e) = self.entries.get_mut(&k) {
                e.brick = None;
            }
            let view = view_at(source, k);
            if content(&view) != Content::Branch {
                continue;
            }
            let wanted = u64::from(slot.alloc.units) * 16;
            if reupload(wanted)
                && let Ok(new_slot) = self.place_brick(k, view, batch)
                && let Some(e) = self.entries.get_mut(&k)
            {
                e.brick = Some(new_slot);
            }
        }

        // Every expanded ancestor rewrites its group, bottom-up.
        let mut groups = std::collections::BTreeSet::new();
        for &k in dirty {
            for level in 2..=root_level {
                let a = k.ancestor(level);
                if self.is_expanded(a) {
                    groups.insert(a);
                }
            }
        }
        for a in groups {
            if !self.entries.contains_key(&a) {
                continue; // dropped by a lower rewrite
            }
            let view = view_at(source, a);
            if content(&view) == Content::Branch {
                let _ = self.write_group(a, view, batch);
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
        self.write_root(source, batch);
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
