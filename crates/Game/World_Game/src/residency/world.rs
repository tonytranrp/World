//! The streamed world: the authoritative tree plus everything that decides
//! which of its chunks are loaded (decision 0006).
//!
//! Every frame [`StreamedWorld::update`] takes the player's position, joins
//! finished chunks to the tree, requests missing ones nearest first, evicts
//! far ones (saving the edited ones) and keeps the coarse tree around the
//! player. Edits go through [`StreamedWorld::apply`] and
//! [`StreamedWorld::fill_box`]: they are logged first, applied at once where
//! their chunk is loaded and kept until it loads everywhere else.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io;
use std::path::Path;
use std::sync::Arc;

use world_core::coords::BrickPos;
use world_core::serialize::region::{REGION_CHUNKS, RegionFile};
use world_core::serialize::wal::{WalRecord, WalWriter};
use world_core::tree::{CHUNK_LEVEL, ChunkData, TreeSnapshot};
use world_core::{Edit, EditError, MaterialId, VoxelBox, VoxelPos, VoxelTree};
use world_gen::{CHUNK_SIZE, ChunkCoord, Generator, GeneratorVersion, TerrainConfig};

use super::config::ResidencyConfig;
use super::jobs::WarmChunk;
use super::plan;
use super::pool::{Done, Pool, Request, Shared};
use super::store::RegionStore;

/// What one loaded chunk is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ChunkState {
    /// Differs from the generated terrain, so it must be saved on eviction.
    edited: bool,
}

/// A logged edit waiting for its chunk to load.
#[derive(Clone, Debug, PartialEq, Eq)]
enum PendingEdit {
    Edits(Vec<Edit>),
    Fill(VoxelBox, MaterialId),
}

/// What changed in the world since the renderer last asked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorldChanges {
    /// Bricks edited in loaded chunks.
    pub bricks: Vec<BrickPos>,
    /// Chunks that joined the tree (every brick of them is new).
    pub loaded: Vec<ChunkCoord>,
    /// Chunks that left the tree (now not resident).
    pub unloaded: Vec<ChunkCoord>,
    /// Whether the coarse tree changed.
    pub coarse: bool,
}

/// Figures for budgets, panels and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResidencyStats {
    pub loaded: usize,
    pub edited: usize,
    pub loading: usize,
    pub saving: usize,
    pub pending_chunks: usize,
    pub hot_bytes: usize,
    pub warm_chunks: usize,
    pub warm_bytes: usize,
    pub coarse_columns: usize,
    pub coarse_bytes: usize,
    pub saved_chunks: usize,
}

/// Compressed edited chunks kept in RAM after eviction, least recently used
/// first out.
#[derive(Default)]
struct Warm {
    entries: BTreeMap<ChunkCoord, (u64, WarmChunk)>,
    bytes: usize,
    tick: u64,
}

impl Warm {
    fn insert(&mut self, coord: ChunkCoord, chunk: WarmChunk, cap: usize) {
        self.tick += 1;
        self.bytes += chunk.size();
        if let Some((_, old)) = self.entries.insert(coord, (self.tick, chunk)) {
            self.bytes -= old.size();
        }
        while self.bytes > cap {
            let Some((&oldest, _)) = self.entries.iter().min_by_key(|(_, (t, _))| *t) else {
                break;
            };
            if let Some((_, old)) = self.entries.remove(&oldest) {
                self.bytes -= old.size();
            }
        }
    }

    fn take(&mut self, coord: ChunkCoord) -> Option<WarmChunk> {
        let (_, chunk) = self.entries.remove(&coord)?;
        self.bytes -= chunk.size();
        Some(chunk)
    }
}

const MIB: usize = 1 << 20;

/// The world tree and its residency.
pub struct StreamedWorld {
    config: ResidencyConfig,
    generator: Arc<Generator>,
    tree: VoxelTree,
    coarse: VoxelTree,
    loaded: BTreeMap<ChunkCoord, ChunkState>,
    loading: BTreeSet<ChunkCoord>,
    /// Edited chunks evicted and not yet written, with their content, so a
    /// quick return reloads them from here.
    saving: BTreeMap<ChunkCoord, ChunkData>,
    /// Loaded chunks waiting for a graft slot.
    ready: VecDeque<(ChunkCoord, ChunkData, bool)>,
    pending: BTreeMap<ChunkCoord, Vec<PendingEdit>>,
    /// Chunks with records in region files.
    saved: BTreeSet<ChunkCoord>,
    warm: Warm,
    coarse_loaded: BTreeMap<(u32, u32), Vec<(ChunkCoord, usize)>>,
    coarse_loading: BTreeSet<(u32, u32)>,
    coarse_bytes: usize,
    pool: Pool,
    wal: WalWriter,
    wal_dirty: bool,
    changes: WorldChanges,
    errors: Vec<String>,
}

/// The chunk-aligned boxes of `region`, one per chunk it touches.
fn split_box(region: VoxelBox) -> Vec<(ChunkCoord, VoxelBox)> {
    if region.is_empty() {
        return Vec::new();
    }
    let (lo, end) = (region.min(), region.end());
    let last = VoxelPos::new(end.x - 1, end.y - 1, end.z - 1);
    let (Some(a), Some(b)) = (ChunkCoord::containing(lo), ChunkCoord::containing(last)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for z in a.z()..=b.z() {
        for y in a.y()..=b.y() {
            for x in a.x()..=b.x() {
                let Ok(c) = ChunkCoord::new(x, y, z) else {
                    continue;
                };
                let cube = VoxelBox::from_min_size(c.min_voxel(), [CHUNK_SIZE; 3]);
                let part = region.intersection(&cube);
                if !part.is_empty() {
                    out.push((c, part));
                }
            }
        }
    }
    out
}

impl StreamedWorld {
    /// A world for `config` and `terrain`, saving under `save_dir`. Reads the
    /// saved-chunk index and replays the edit log, so edits made before a
    /// crash wait for their chunks again.
    pub fn open(
        config: ResidencyConfig,
        terrain: TerrainConfig,
        save_dir: &Path,
    ) -> io::Result<Self> {
        let generator = Generator::new(config.seed, GeneratorVersion::CURRENT, terrain)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
        let generator = Arc::new(generator);
        let store = RegionStore::open(save_dir)?;
        let saved = scan_saved(&store);
        let (wal, replay) = WalWriter::open(&store.wal_path())?;
        let shared = Shared {
            generator: Arc::clone(&generator),
            generator_version: GeneratorVersion::CURRENT.0,
            coarse_level: config.coarse_cell_level,
        };
        let pool = Pool::new(shared, store, config.workers);
        let mut world = Self {
            config,
            generator,
            tree: VoxelTree::unloaded(),
            coarse: VoxelTree::new(),
            loaded: BTreeMap::new(),
            loading: BTreeSet::new(),
            saving: BTreeMap::new(),
            ready: VecDeque::new(),
            pending: BTreeMap::new(),
            saved,
            warm: Warm::default(),
            coarse_loaded: BTreeMap::new(),
            coarse_loading: BTreeSet::new(),
            coarse_bytes: 0,
            pool,
            wal,
            wal_dirty: false,
            changes: WorldChanges::default(),
            errors: Vec::new(),
        };
        for record in replay {
            match record {
                WalRecord::Edits(edits) => world.queue_edits(&edits),
                WalRecord::Fill { region, material } => {
                    for (c, part) in split_box(region) {
                        world
                            .pending
                            .entry(c)
                            .or_default()
                            .push(PendingEdit::Fill(part, material));
                    }
                }
                WalRecord::Checkpoint(_) => {}
            }
        }
        Ok(world)
    }

    pub fn generator(&self) -> &Generator {
        &self.generator
    }

    /// The full-detail world as it is now, for the renderer and queries.
    pub fn snapshot(&self) -> TreeSnapshot {
        self.tree.snapshot()
    }

    /// The coarse world beyond the full-detail radius.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "read by the World_Render feed once the GPU cache is merged"
        )
    )]
    pub fn coarse_snapshot(&self) -> TreeSnapshot {
        self.coarse.snapshot()
    }

    /// What changed since the last call.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "read by the World_Render feed once the GPU cache is merged"
        )
    )]
    pub fn take_changes(&mut self) -> WorldChanges {
        std::mem::take(&mut self.changes)
    }

    /// Problems met while loading or saving (unreadable records, failed
    /// writes), since the last call.
    pub fn take_errors(&mut self) -> Vec<String> {
        std::mem::take(&mut self.errors)
    }

    pub fn stats(&self) -> ResidencyStats {
        ResidencyStats {
            loaded: self.loaded.len(),
            edited: self.loaded.values().filter(|s| s.edited).count(),
            loading: self.loading.len(),
            saving: self.saving.len(),
            pending_chunks: self.pending.len(),
            hot_bytes: self.tree.stats().bytes,
            warm_chunks: self.warm.entries.len(),
            warm_bytes: self.warm.bytes,
            coarse_columns: self.coarse_loaded.len(),
            coarse_bytes: self.coarse_bytes,
            saved_chunks: self.saved.len(),
        }
    }

    /// Whether every chunk within the full-detail radius of `eye` is loaded.
    #[cfg(test)]
    pub fn is_settled(&self, eye: VoxelPos) -> bool {
        plan::sphere(eye, self.config.full_radius_m)
            .iter()
            .all(|c| self.loaded.contains_key(c))
    }

    /// One frame of residency around `eye`.
    pub fn update(&mut self, eye: VoxelPos) {
        for done in self.pool.drain() {
            self.finish(done, eye);
        }
        self.graft_ready();
        self.request_loads(eye);
        self.evict(eye);
        self.update_coarse(eye);
        if self.wal_dirty {
            if let Err(e) = self.wal.sync() {
                self.errors.push(format!("edit log sync: {e}"));
            }
            self.wal_dirty = false;
        }
    }

    /// Applies a batch of edits: logged first, applied now where the chunk is
    /// loaded, kept for later where it is not. Returns the bricks changed now.
    pub fn apply(&mut self, edits: &[Edit]) -> Result<Vec<BrickPos>, EditError> {
        if let Some(bad) = edits.iter().find(|e| !e.pos.in_world()) {
            return Err(EditError::OutOfWorld { pos: bad.pos });
        }
        self.log(&WalRecord::Edits(edits.to_vec()));
        let mut now = Vec::new();
        let mut later: BTreeMap<ChunkCoord, Vec<Edit>> = BTreeMap::new();
        for &edit in edits {
            match ChunkCoord::containing(edit.pos) {
                Some(c) if self.loaded.contains_key(&c) => now.push(edit),
                Some(c) => later.entry(c).or_default().push(edit),
                None => {}
            }
        }
        for (c, edits) in later {
            self.pending
                .entry(c)
                .or_default()
                .push(PendingEdit::Edits(edits));
        }
        let bricks = self.tree.apply(&now)?;
        for &edit in &now {
            if let Some(c) = ChunkCoord::containing(edit.pos)
                && let Some(state) = self.loaded.get_mut(&c)
            {
                state.edited = true;
            }
        }
        self.changes.bricks.extend_from_slice(bricks.as_slice());
        Ok(bricks.as_slice().to_vec())
    }

    /// Fills a box, with the same rules as [`apply`](Self::apply).
    pub fn fill_box(&mut self, region: VoxelBox, material: MaterialId) -> Result<(), EditError> {
        let parts = split_box(region);
        if parts.is_empty() && !region.is_empty() {
            return Err(EditError::OutOfWorld { pos: region.min() });
        }
        self.log(&WalRecord::Fill { region, material });
        for (c, part) in parts {
            if self.loaded.contains_key(&c) {
                self.tree.fill_box(part, material)?;
                if let Some(state) = self.loaded.get_mut(&c) {
                    state.edited = true;
                }
                self.mark_bricks(part);
            } else {
                self.pending
                    .entry(c)
                    .or_default()
                    .push(PendingEdit::Fill(part, material));
            }
        }
        Ok(())
    }

    /// Saves every edited chunk, loaded or waiting, and empties the edit log.
    /// Blocks until the region files are written.
    pub fn save_all(&mut self) -> io::Result<()> {
        // Chunks with waiting edits are loaded here, so their edits land in
        // their region files.
        let waiting: Vec<ChunkCoord> = self
            .pending
            .keys()
            .copied()
            .filter(|c| !self.loaded.contains_key(c))
            .collect();
        for done in self.pool.wait_idle() {
            self.finish_without_eye(done);
        }
        for c in waiting {
            if self.saving.contains_key(&c) {
                continue;
            }
            self.pool.submit(Request::Load {
                coord: c,
                warm: self.warm.take(c),
                saved: self.saved.contains(&c),
            });
            self.loading.insert(c);
        }
        for done in self.pool.wait_idle() {
            self.finish_without_eye(done);
        }
        self.graft_all_ready();
        let edited: Vec<ChunkCoord> = self
            .loaded
            .iter()
            .filter(|(_, s)| s.edited)
            .map(|(&c, _)| c)
            .collect();
        for c in edited {
            if let Some(data) = self.tree.chunk_data(c.min_voxel()) {
                self.pool.submit(Request::Save { coord: c, data });
            }
        }
        for done in self.pool.wait_idle() {
            self.finish_without_eye(done);
        }
        if !self.errors.is_empty() {
            return Err(io::Error::other(self.errors.join("; ")));
        }
        self.wal.reset()
    }

    fn log(&mut self, record: &WalRecord) {
        if let Err(e) = self.wal.append(record) {
            self.errors.push(format!("edit log append: {e}"));
        }
        self.wal_dirty = true;
    }

    fn queue_edits(&mut self, edits: &[Edit]) {
        let mut by_chunk: BTreeMap<ChunkCoord, Vec<Edit>> = BTreeMap::new();
        for &e in edits {
            if let Some(c) = ChunkCoord::containing(e.pos) {
                by_chunk.entry(c).or_default().push(e);
            }
        }
        for (c, edits) in by_chunk {
            self.pending
                .entry(c)
                .or_default()
                .push(PendingEdit::Edits(edits));
        }
    }

    fn mark_bricks(&mut self, region: VoxelBox) {
        let (lo, end) = (region.min(), region.end());
        let first = lo.brick();
        let last = VoxelPos::new(end.x - 1, end.y - 1, end.z - 1).brick();
        for z in first.z..=last.z {
            for y in first.y..=last.y {
                for x in first.x..=last.x {
                    self.changes.bricks.push(BrickPos::new(x, y, z));
                }
            }
        }
    }

    fn finish_without_eye(&mut self, done: Done) {
        // Saving keeps every loaded chunk, wherever the player is.
        self.finish(done, VoxelPos::new(i32::MIN, i32::MIN, i32::MIN));
    }

    fn finish(&mut self, done: Done, eye: VoxelPos) {
        match done {
            Done::Loaded {
                coord,
                data,
                edited,
                error,
            } => {
                self.loading.remove(&coord);
                if let Some(e) = error {
                    self.errors.push(e);
                }
                let wanted = eye.x == i32::MIN
                    || plan::chunk_distance(eye, coord) <= plan::voxels(self.config.evict_radius_m)
                    || self.pending.contains_key(&coord);
                if wanted && !self.loaded.contains_key(&coord) {
                    self.ready.push_back((coord, data, edited));
                }
            }
            Done::Saved { coord, warm, error } => {
                self.saving.remove(&coord);
                match error {
                    Some(e) => self.errors.push(format!("saving chunk {coord:?}: {e}")),
                    None => {
                        self.saved.insert(coord);
                        self.warm
                            .insert(coord, warm, self.config.warm_cap_mib as usize * MIB);
                    }
                }
            }
            Done::Coarse { x, z, chunks } => {
                self.coarse_loading.remove(&(x, z));
                let wanted = eye.x == i32::MIN
                    || plan::column_distance(eye, x, z)
                        <= plan::voxels(self.config.coarse_radius_m) * 1.25;
                if !wanted || self.coarse_loaded.contains_key(&(x, z)) {
                    return;
                }
                let mut entry = Vec::new();
                for (coord, data) in chunks {
                    let bytes = data.bytes();
                    if self.coarse.insert_chunk(coord.min_voxel(), data).is_ok() {
                        entry.push((coord, bytes));
                        self.coarse_bytes += bytes;
                    }
                }
                self.coarse_loaded.insert((x, z), entry);
                self.changes.coarse = true;
            }
        }
    }

    fn graft_ready(&mut self) {
        for _ in 0..self.config.grafts_per_frame {
            let Some((coord, data, edited)) = self.ready.pop_front() else {
                break;
            };
            self.join(coord, data, edited);
        }
    }

    fn graft_all_ready(&mut self) {
        while let Some((coord, data, edited)) = self.ready.pop_front() {
            self.join(coord, data, edited);
        }
    }

    /// Joins a loaded chunk to the tree and applies its waiting edits.
    fn join(&mut self, coord: ChunkCoord, data: ChunkData, edited: bool) {
        let min = coord.min_voxel();
        if self.tree.insert_chunk(min, data).is_err() {
            return;
        }
        let mut state = ChunkState { edited };
        for edit in self.pending.remove(&coord).unwrap_or_default() {
            let result = match edit {
                PendingEdit::Edits(edits) => self.tree.apply(&edits).map(|_| ()),
                PendingEdit::Fill(region, m) => self.tree.fill_box(region, m).map(|_| ()),
            };
            match result {
                Ok(()) => state.edited = true,
                Err(e) => self.errors.push(format!("waiting edit for {coord:?}: {e}")),
            }
        }
        self.loaded.insert(coord, state);
        self.changes.loaded.push(coord);
    }

    fn request_loads(&mut self, eye: VoxelPos) {
        let in_flight_cap = (self.config.workers.max(1) as usize) * 4;
        for coord in plan::sphere(eye, self.config.full_radius_m) {
            if self.loaded.contains_key(&coord)
                || self.loading.contains(&coord)
                || self.ready.iter().any(|(c, _, _)| *c == coord)
            {
                continue;
            }
            if let Some(data) = self.saving.get(&coord).cloned() {
                // Evicted a moment ago and not yet written: rejoin as is.
                self.ready.push_back((coord, data, true));
                continue;
            }
            if self.loading.len() >= in_flight_cap {
                break;
            }
            self.loading.insert(coord);
            self.pool.submit(Request::Load {
                coord,
                warm: self.warm.take(coord),
                saved: self.saved.contains(&coord),
            });
        }
    }

    fn evict(&mut self, eye: VoxelPos) {
        let far = plan::voxels(self.config.evict_radius_m);
        let mut doomed: Vec<ChunkCoord> = self
            .loaded
            .keys()
            .copied()
            .filter(|&c| plan::chunk_distance(eye, c) > far)
            .collect();
        // Over the RAM cap: also evict the farthest chunks outside the
        // full-detail radius until the estimate fits.
        let cap = self.config.hot_cap_mib as usize * MIB;
        let mut bytes = self.tree.stats().bytes;
        if bytes > cap {
            let full = plan::voxels(self.config.full_radius_m);
            let mut extra: Vec<(u32, ChunkCoord)> = self
                .loaded
                .keys()
                .copied()
                .filter(|c| !doomed.contains(c))
                .map(|c| (plan::chunk_distance(eye, c).to_bits(), c))
                .filter(|&(d, _)| f32::from_bits(d) > full)
                .collect();
            extra.sort_unstable_by(|a, b| b.cmp(a));
            for (_, c) in extra {
                if bytes <= cap {
                    break;
                }
                bytes = bytes
                    .saturating_sub(self.tree.chunk_data(c.min_voxel()).map_or(0, |d| d.bytes()));
                doomed.push(c);
            }
        }
        for c in doomed {
            self.unload(c);
        }
    }

    fn unload(&mut self, coord: ChunkCoord) {
        let Some(state) = self.loaded.remove(&coord) else {
            return;
        };
        let Ok(data) = self.tree.take_chunk(coord.min_voxel()) else {
            return;
        };
        if state.edited {
            self.saving.insert(coord, data.clone());
            self.pool.submit(Request::Save { coord, data });
        }
        self.changes.unloaded.push(coord);
    }

    fn update_coarse(&mut self, eye: VoxelPos) {
        if self.config.coarse_radius_m <= 0.0 {
            return;
        }
        let in_flight_cap = (self.config.workers.max(1) as usize) * 2;
        for (x, z) in plan::disc(eye, self.config.coarse_radius_m) {
            if self.coarse_loaded.contains_key(&(x, z)) || self.coarse_loading.contains(&(x, z)) {
                continue;
            }
            if self.coarse_loading.len() >= in_flight_cap {
                break;
            }
            let edited: Vec<ChunkCoord> = self
                .saved
                .iter()
                .copied()
                .filter(|c| c.x() == x && c.z() == z)
                .collect();
            self.coarse_loading.insert((x, z));
            self.pool.submit(Request::Coarse { x, z, edited });
        }
        // Drop columns beyond a margin, then the farthest while over the cap.
        let keep = plan::voxels(self.config.coarse_radius_m) * 1.25;
        let cap = self.config.coarse_cap_mib as usize * MIB;
        let mut columns: Vec<(u32, (u32, u32))> = self
            .coarse_loaded
            .keys()
            .map(|&(x, z)| (plan::column_distance(eye, x, z).to_bits(), (x, z)))
            .collect();
        columns.sort_unstable_by(|a, b| b.cmp(a));
        for (d, column) in columns {
            if f32::from_bits(d) <= keep && self.coarse_bytes <= cap {
                break;
            }
            if let Some(chunks) = self.coarse_loaded.remove(&column) {
                for (coord, bytes) in chunks {
                    let _ = self.coarse.unload(CHUNK_LEVEL, coord.min_voxel());
                    self.coarse_bytes = self.coarse_bytes.saturating_sub(bytes);
                }
                self.changes.coarse = true;
            }
        }
    }
}

/// Every chunk with records in the store's region files.
fn scan_saved(store: &RegionStore) -> BTreeSet<ChunkCoord> {
    let mut saved = BTreeSet::new();
    let Ok(entries) = std::fs::read_dir(store.dir()) else {
        return saved;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "wvr") {
            continue;
        }
        let Ok(file) = RegionFile::open(&path) else {
            continue;
        };
        let r = file.index().region;
        for e in &file.index().entries {
            let c = [0, 1, 2].map(|a| r[a] * REGION_CHUNKS + e.chunk[a]);
            if let Ok(coord) = ChunkCoord::new(c[0], c[1], c[2]) {
                saved.insert(coord);
            }
        }
    }
    saved
}
