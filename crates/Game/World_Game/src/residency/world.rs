//! The streamed world: the authoritative tree plus everything that decides
//! which of its chunks are loaded (decision 0006).
//!
//! Every frame [`StreamedWorld::update`] takes the player's position, joins
//! finished chunks to the tree, requests missing ones nearest first, evicts
//! far ones (saving the edited ones) and keeps the coarse tree around the
//! player. Edits go through [`StreamedWorld::apply`] and
//! [`StreamedWorld::fill_box`]: they are logged first, applied at once where
//! their chunk is loaded and kept until it loads everywhere else.
//!
//! When the edit log grows past `log_checkpoint_kib`, a checkpoint writes
//! every chunk with unsaved edits to its region file without stopping the
//! game, then restarts the log from a `Checkpoint` record followed by what is
//! still only in the log: the edits waiting for unloaded chunks and every
//! record logged while the checkpoint ran.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use world_core::coords::BrickPos;
use world_core::serialize::region::{REGION_CHUNKS, RegionFile};
use world_core::serialize::wal::{WalRecord, WalWriter};
use world_core::tree::{CHUNK_LEVEL, ChunkData, TreeSnapshot};
use world_core::{Edit, EditError, MaterialId, VoxelBox, VoxelPos, VoxelTree};
use world_gen::{CHUNK_SIZE, ChunkCoord, Generator, GeneratorVersion, TerrainConfig};

use super::columns::ColumnCache;
use super::config::ResidencyConfig;
use super::jobs::{Generators, LoadSource, WarmChunk};
use super::plan::{self, Heights, Motion};
use super::pool::{Done, Pool, Request, Shared};
use super::store::RegionStore;
use super::warm::Warm;

/// What one loaded chunk is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ChunkState {
    /// Holds changes that are not in its region file yet, so it must be
    /// saved before it is dropped.
    edited: bool,
    /// Bytes its pages hold, kept up to date so the hot cap needs no walk.
    bytes: usize,
}

/// A logged edit waiting for its chunk to load.
#[derive(Clone, Debug, PartialEq, Eq)]
enum PendingEdit {
    Edits(Vec<Edit>),
    Fill(VoxelBox, MaterialId),
}

/// One chunk's region file write, from submission until it lands.
#[derive(Default)]
struct Write {
    /// The evicted content and its bytes, kept until it is written so the
    /// renderer still draws it and a quick return rejoins it. `None` while
    /// the chunk is loaded (or about to join).
    held: Option<(ChunkData, usize)>,
    /// A write is in flight.
    in_flight: bool,
    /// Newer content to write once the one in flight lands; one write per
    /// chunk at a time keeps the region file from going back in time.
    next: Option<ChunkData>,
}

/// A checkpoint of the edit log in progress.
struct Checkpoint {
    sequence: u64,
    /// Chunks whose writes must land before the log can restart.
    waiting: BTreeSet<ChunkCoord>,
    /// The edits waiting for unloaded chunks when it started, which are
    /// only in the log.
    relog: Vec<WalRecord>,
    /// Everything logged since it started.
    tail: Vec<WalRecord>,
}

/// What changed in the world since the renderer last asked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorldChanges {
    /// Bricks of the render tree whose content changed by edits.
    pub render_bricks: Vec<BrickPos>,
    /// The lowest voxel of every chunk of the render tree that was swapped
    /// (loaded, unloaded, or switched between full and coarse), which makes
    /// the GPU cache rebuild that region from the snapshot.
    pub render_chunks: Vec<VoxelPos>,
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
    /// Chunks with a region file write in flight or waiting.
    pub saving: usize,
    /// Bytes of evicted chunks held until their writes land; they count
    /// toward the hot cap.
    pub saving_bytes: usize,
    pub pending_chunks: usize,
    /// Bytes of the loaded chunks.
    pub hot_bytes: usize,
    pub warm_chunks: usize,
    /// Of those, the ones holding saved edits.
    pub warm_edited: usize,
    pub warm_bytes: usize,
    /// Unchanged chunks waiting to be compressed for the warm cache or
    /// being compressed.
    pub compressing: usize,
    /// Bytes of the chunks waiting to be compressed; they count toward the
    /// hot cap.
    pub compressing_bytes: usize,
    pub coarse_columns: usize,
    pub coarse_bytes: usize,
    pub saved_chunks: usize,
    /// Chunk writes submitted since the world opened.
    pub writes: usize,
    /// Edit log checkpoints completed since the world opened.
    pub checkpoints: usize,
    /// Bytes in the edit log.
    pub log_bytes: u64,
    /// Chunks within the full-detail radius that were not loaded at the last
    /// update.
    pub holes: usize,
    /// Of those, the ones ahead of the player's motion.
    pub holes_ahead: usize,
    /// `holes` and `holes_ahead` summed over every update, and the number
    /// of updates, for averages over an interval.
    pub hole_sum: u64,
    pub hole_ahead_sum: u64,
    pub updates: u64,
    /// Chunk loads since the world opened, by where their content came from.
    pub generated: usize,
    pub warm_hits: usize,
    pub disk_loads: usize,
    /// Worker time spent on generated loads since the world opened, in
    /// microseconds, and the longest one.
    pub generated_micros: u64,
    pub generated_max_micros: u64,
    /// Of the generated loads, those proven one material (air above the
    /// terrain, bedrock below the caves) and not built.
    pub uniform_loads: usize,
    /// Column footprints served by the shared cache, and computed.
    pub column_hits: u64,
    pub column_misses: u64,
}

const MIB: usize = 1 << 20;

/// The world tree and its residency.
pub struct StreamedWorld {
    config: ResidencyConfig,
    generator: Arc<Generator>,
    tree: VoxelTree,
    coarse: VoxelTree,
    /// What the renderer draws: the loaded chunks of `tree` (shared, not
    /// copied), evicted chunks still being written, and elsewhere the coarse
    /// chunks.
    render: VoxelTree,
    /// The coarse chunks by coordinate, to fill the render tree.
    coarse_chunks: BTreeMap<ChunkCoord, ChunkData>,
    loaded: BTreeMap<ChunkCoord, ChunkState>,
    /// Sum of `bytes` over `loaded`.
    hot_bytes: usize,
    loading: BTreeSet<ChunkCoord>,
    /// Region file writes by chunk. A chunk with an entry is never read
    /// from disk: its newest content is loaded, joining, or held here.
    writes: BTreeMap<ChunkCoord, Write>,
    /// Sum of the held bytes in `writes`.
    held_bytes: usize,
    /// Loaded chunks waiting for a graft slot, and whether they hold changes
    /// that may not be on disk.
    ready: VecDeque<(ChunkCoord, ChunkData, bool)>,
    pending: BTreeMap<ChunkCoord, Vec<PendingEdit>>,
    /// Chunks with records in region files.
    saved: BTreeSet<ChunkCoord>,
    /// Chunks with a saved record that could not be loaded; it is set aside
    /// before the chunk is written.
    refused: BTreeSet<ChunkCoord>,
    warm: Warm,
    /// Unchanged chunks waiting to be compressed for the warm cache, or
    /// being compressed, with the ticket of the newest request. A write that
    /// lands makes the requests before it stale (their content is older
    /// than the region file).
    compressing: BTreeMap<ChunkCoord, u64>,
    tickets: u64,
    /// Compressions waiting for idle workers, oldest first, with their
    /// tickets and bytes. Loads come first: a chunk is compressed only
    /// while fewer loads than the in-flight cap are outstanding, and the
    /// oldest waiting ones are dropped (not kept warm) past a bound.
    to_compress: VecDeque<(ChunkCoord, ChunkData, u64, usize)>,
    /// Sum of the bytes in `to_compress`; they count toward the hot cap.
    to_compress_bytes: usize,
    compress_in_flight: usize,
    coarse_loaded: BTreeMap<(u32, u32), Vec<(ChunkCoord, usize)>>,
    coarse_loading: BTreeSet<(u32, u32)>,
    /// Columns being built while one of their chunks was written; their
    /// result is stale and they are requested again.
    coarse_stale: BTreeSet<(u32, u32)>,
    coarse_bytes: usize,
    pool: Pool,
    wal: WalWriter,
    wal_path: PathBuf,
    wal_dirty: bool,
    /// The log's length after the last sync.
    wal_bytes: u64,
    /// The log length that starts the next checkpoint.
    checkpoint_at: u64,
    checkpoint: Option<Checkpoint>,
    checkpoint_sequence: u64,
    checkpoints: usize,
    writes_submitted: usize,
    /// Chunk writes that failed since the world opened.
    write_failures: usize,
    changes: WorldChanges,
    errors: Vec<String>,
    motion: Motion,
    /// Load and hole figures for [`ResidencyStats`].
    counters: Counters,
    /// Column footprints shared with the workers.
    columns: Arc<ColumnCache>,
    /// The terrain heights of chunk columns, from full-detail loads (exact)
    /// and coarse columns (sampled), to put buried chunks after visible
    /// ones.
    heights: BTreeMap<(u32, u32), Heights>,
}

/// Column heights kept at most; beyond it the farthest are dropped.
const MAX_HEIGHTS: usize = 8192;

/// Running figures reported in [`ResidencyStats`].
#[derive(Clone, Copy, Debug, Default)]
struct Counters {
    holes: usize,
    holes_ahead: usize,
    hole_sum: u64,
    hole_ahead_sum: u64,
    updates: u64,
    generated: usize,
    warm_hits: usize,
    disk_loads: usize,
    generated_micros: u64,
    generated_max_micros: u64,
    uniform_loads: usize,
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

/// The length of the file at `path`, 0 if it cannot be read.
fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |m| m.len())
}

impl StreamedWorld {
    /// A world for `config` and `terrain`, saving under `save_dir`. Reads the
    /// saved-chunk index and replays the edit log from its last checkpoint,
    /// so edits made before a crash wait for their chunks again.
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
        let wal_path = store.wal_path();
        let (wal, replay) = WalWriter::open(&wal_path)?;
        let wal_bytes = file_len(&wal_path);
        let columns = Arc::new(ColumnCache::new(config.column_cache_mib));
        let shared = Shared {
            generators: Arc::new(Generators::new(Arc::clone(&generator))),
            coarse_level: config.coarse_cell_level,
            columns: Arc::clone(&columns),
        };
        let pool = Pool::new(shared, store, config.workers);
        let checkpoint_at = config.log_checkpoint_kib as u64 * 1024;
        let mut world = Self {
            config,
            generator,
            tree: VoxelTree::unloaded(),
            coarse: VoxelTree::new(),
            render: VoxelTree::new(),
            coarse_chunks: BTreeMap::new(),
            loaded: BTreeMap::new(),
            hot_bytes: 0,
            loading: BTreeSet::new(),
            writes: BTreeMap::new(),
            held_bytes: 0,
            ready: VecDeque::new(),
            pending: BTreeMap::new(),
            saved,
            refused: BTreeSet::new(),
            warm: Warm::default(),
            compressing: BTreeMap::new(),
            tickets: 0,
            to_compress: VecDeque::new(),
            to_compress_bytes: 0,
            compress_in_flight: 0,
            coarse_loaded: BTreeMap::new(),
            coarse_loading: BTreeSet::new(),
            coarse_stale: BTreeSet::new(),
            coarse_bytes: 0,
            pool,
            wal,
            wal_path,
            wal_dirty: false,
            wal_bytes,
            checkpoint_at,
            checkpoint: None,
            checkpoint_sequence: 0,
            checkpoints: 0,
            writes_submitted: 0,
            write_failures: 0,
            changes: WorldChanges::default(),
            errors: Vec::new(),
            motion: Motion::default(),
            counters: Counters::default(),
            columns,
            heights: BTreeMap::new(),
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
                // Everything before it is in region files.
                WalRecord::Checkpoint(sequence) => {
                    world.pending.clear();
                    world.checkpoint_sequence = world.checkpoint_sequence.max(sequence);
                }
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

    /// What the renderer draws: full detail where loaded, coarse cells
    /// elsewhere in the coarse ring, air beyond.
    pub fn render_snapshot(&self) -> TreeSnapshot {
        self.render.snapshot()
    }

    /// Puts the current content of chunk `coord` into the render tree: the
    /// loaded chunk, else the evicted content still being written, else its
    /// coarse chunk, else air.
    fn refresh_render(&mut self, coord: ChunkCoord) {
        let min = coord.min_voxel();
        let data = if self.loaded.contains_key(&coord) {
            self.tree.chunk_data(min)
        } else if let Some((data, _)) = self.writes.get(&coord).and_then(|w| w.held.as_ref()) {
            Some(data.clone())
        } else {
            self.coarse_chunks.get(&coord).cloned()
        };
        let result = match data {
            Some(data) => self.render.insert_chunk(min, data),
            None => self.render.take_chunk(min).map(|_| true),
        };
        if result.is_ok() {
            self.changes.render_chunks.push(min);
        }
    }

    /// The coarse world beyond the full-detail radius.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the renderer draws the render tree; the coarse tree is for queries and tests"
        )
    )]
    pub fn coarse_snapshot(&self) -> TreeSnapshot {
        self.coarse.snapshot()
    }

    /// What changed since the last call.
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
            saving: self.writes.len(),
            saving_bytes: self.held_bytes,
            pending_chunks: self.pending.len(),
            hot_bytes: self.hot_bytes,
            warm_chunks: self.warm.len(),
            warm_edited: self.warm.edited(),
            warm_bytes: self.warm.bytes(),
            compressing: self.to_compress.len() + self.compress_in_flight,
            compressing_bytes: self.to_compress_bytes,
            coarse_columns: self.coarse_loaded.len(),
            coarse_bytes: self.coarse_bytes,
            saved_chunks: self.saved.len(),
            writes: self.writes_submitted,
            checkpoints: self.checkpoints,
            log_bytes: self.wal_bytes,
            holes: self.counters.holes,
            holes_ahead: self.counters.holes_ahead,
            hole_sum: self.counters.hole_sum,
            hole_ahead_sum: self.counters.hole_ahead_sum,
            updates: self.counters.updates,
            generated: self.counters.generated,
            warm_hits: self.counters.warm_hits,
            disk_loads: self.counters.disk_loads,
            generated_micros: self.counters.generated_micros,
            generated_max_micros: self.counters.generated_max_micros,
            uniform_loads: self.counters.uniform_loads,
            column_hits: self.columns.counts().hits,
            column_misses: self.columns.counts().misses,
        }
    }

    /// The player's smoothed velocity, in metres per second.
    pub fn velocity_m_s(&self) -> [f32; 3] {
        self.motion
            .velocity()
            .map(|v| v / world_core::consts::VOXELS_PER_METER as f32)
    }

    /// Whether every chunk within the full-detail radius of `eye` is loaded.
    pub fn is_settled(&self, eye: VoxelPos) -> bool {
        plan::sphere(eye, self.config.full_radius_m)
            .iter()
            .all(|c| self.loaded.contains_key(c))
    }

    /// One frame of residency around `eye`, `dt_secs` after the previous
    /// one (successive positions give the player's velocity).
    pub fn update(&mut self, eye: VoxelPos, dt_secs: f32) {
        self.motion.observe(
            eye,
            dt_secs,
            self.config.velocity_smoothing_secs,
            plan::voxels(self.config.evict_radius_m),
        );
        for done in self.pool.drain() {
            self.finish(done, eye);
        }
        self.graft_ready();
        self.request_loads(eye);
        self.evict(eye);
        self.submit_compressions();
        self.update_coarse(eye);
        if self.wal_dirty {
            if let Err(e) = self.wal.sync() {
                self.errors.push(format!("edit log sync: {e}"));
            }
            self.wal_dirty = false;
            self.wal_bytes = file_len(&self.wal_path);
        }
        if self.checkpoint.is_none() && self.wal_bytes > self.checkpoint_at {
            self.start_checkpoint();
        }
        self.finish_checkpoint();
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
        // Only chunks whose content changed need saving.
        let mut touched = BTreeSet::new();
        for brick in bricks.as_slice() {
            if let Some(c) = ChunkCoord::containing(brick.min_voxel())
                && let Some(state) = self.loaded.get_mut(&c)
            {
                state.edited = true;
                touched.insert(c);
            }
        }
        for &c in &touched {
            self.refresh_bytes(c);
        }
        self.share_edits(&touched, bricks.as_slice());
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
                if !self.tree.fill_box(part, material)? {
                    continue;
                }
                if let Some(state) = self.loaded.get_mut(&c) {
                    state.edited = true;
                }
                self.refresh_bytes(c);
                let before = self.changes.bricks.len();
                self.mark_bricks(part);
                let marked: Vec<BrickPos> = self.changes.bricks[before..].to_vec();
                self.share_edits(&BTreeSet::from([c]), &marked);
            } else {
                self.pending
                    .entry(c)
                    .or_default()
                    .push(PendingEdit::Fill(part, material));
            }
        }
        Ok(())
    }

    /// Saves every edited chunk, loaded, evicted or waiting, and empties the
    /// edit log. Blocks until the region files are written.
    pub fn save_all(&mut self) -> io::Result<()> {
        let errors_before = self.errors.len();
        self.drain_all();
        // Writes that failed earlier are tried again; from here on any
        // failure keeps the log.
        let failures_before = self.write_failures;
        self.retry_failed_writes();
        self.drain_all();
        // Chunks with waiting edits are loaded here, so their edits land in
        // their region files.
        let waiting: Vec<ChunkCoord> = self
            .pending
            .keys()
            .copied()
            .filter(|c| !self.loaded.contains_key(c))
            .collect();
        for c in waiting {
            if self.loading.contains(&c) || self.ready.iter().any(|(r, _, _)| *r == c) {
                continue;
            }
            if let Some(w) = self.writes.get_mut(&c) {
                // Nothing is in flight after the drain: the entry is a failed
                // write, and its content now rejoins (and is written below).
                if let Some((data, bytes)) = w.held.take() {
                    self.held_bytes -= bytes;
                    self.ready.push_back((c, data, true));
                }
                if !w.in_flight {
                    self.writes.remove(&c);
                }
                continue;
            }
            self.loading.insert(c);
            self.pool.submit(Request::Load {
                coord: c,
                warm: self.warm.get(c),
                saved: self.saved.contains(&c),
            });
        }
        self.drain_all();
        self.graft_all_ready();
        self.save_edited();
        self.drain_all();
        // The log is the only copy of anything not written: it is kept
        // unless every change reached a region file.
        let unsaved = self.loaded.values().filter(|s| s.edited).count();
        if self.write_failures != failures_before
            || !self.writes.is_empty()
            || unsaved > 0
            || !self.pending.is_empty()
        {
            let errors = self.errors[errors_before.min(self.errors.len())..].join("; ");
            return Err(io::Error::other(format!(
                "the edit log is kept: {} failed writes, {} writes outstanding, {} edited \
                 chunks unsaved, {} chunks with waiting edits: {errors}",
                self.write_failures - failures_before,
                self.writes.len(),
                unsaved,
                self.pending.len()
            )));
        }
        self.wal.reset()?;
        self.wal_bytes = file_len(&self.wal_path);
        self.checkpoint = None;
        self.checkpoint_at = self.wal_bytes + self.checkpoint_size();
        Ok(())
    }

    /// Waits until no request is in flight, finishing each result (which
    /// may submit queued writes).
    fn drain_all(&mut self) {
        loop {
            let done = self.pool.wait_idle();
            if done.is_empty() {
                break;
            }
            for d in done {
                self.finish_without_eye(d);
            }
        }
    }

    /// Submits a write for every loaded chunk with unsaved changes, which
    /// then counts as saved (a failed write marks it edited again).
    fn save_edited(&mut self) {
        let edited: Vec<ChunkCoord> = self
            .loaded
            .iter()
            .filter(|(_, s)| s.edited)
            .map(|(&c, _)| c)
            .collect();
        for c in edited {
            if let Some(data) = self.tree.chunk_data(c.min_voxel()) {
                if let Some(state) = self.loaded.get_mut(&c) {
                    state.edited = false;
                }
                self.save(c, data, false);
            }
        }
    }

    /// Submits again every write that failed and still holds its content.
    fn retry_failed_writes(&mut self) {
        let failed: Vec<(ChunkCoord, ChunkData)> = self
            .writes
            .iter()
            .filter(|(_, w)| !w.in_flight)
            .filter_map(|(&c, w)| w.held.as_ref().map(|(d, _)| (c, d.clone())))
            .collect();
        for (c, data) in failed {
            if let Some(w) = self.writes.get_mut(&c) {
                w.in_flight = true;
            }
            self.submit_write(c, data);
        }
    }

    /// Writes `data` as the content of chunk `coord`, after any write of it
    /// already in flight. With `hold`, the chunk was evicted and `data` is
    /// kept until it lands.
    fn save(&mut self, coord: ChunkCoord, data: ChunkData, hold: bool) {
        let w = self.writes.entry(coord).or_default();
        if hold {
            let bytes = data.bytes();
            if let Some((_, old)) = w.held.replace((data.clone(), bytes)) {
                self.held_bytes -= old;
            }
            self.held_bytes += bytes;
        }
        if w.in_flight {
            w.next = Some(data);
        } else {
            w.in_flight = true;
            self.submit_write(coord, data);
        }
    }

    fn submit_write(&mut self, coord: ChunkCoord, data: ChunkData) {
        self.writes_submitted += 1;
        self.pool.submit(Request::Save {
            coord,
            data,
            set_aside: self.refused.contains(&coord),
        });
    }

    /// The log growth that starts a checkpoint, in bytes.
    fn checkpoint_size(&self) -> u64 {
        self.config.log_checkpoint_kib as u64 * 1024
    }

    /// Starts writing every chunk whose changes are only in the log.
    fn start_checkpoint(&mut self) {
        // Chunks that rejoined with unsaved changes are loaded first, so
        // they are written with the rest.
        self.graft_all_ready();
        self.retry_failed_writes();
        self.save_edited();
        let mut relog = Vec::new();
        for edits in self.pending.values() {
            for edit in edits {
                relog.push(match edit {
                    PendingEdit::Edits(edits) => WalRecord::Edits(edits.clone()),
                    PendingEdit::Fill(region, material) => WalRecord::Fill {
                        region: *region,
                        material: *material,
                    },
                });
            }
        }
        self.checkpoint_sequence += 1;
        self.checkpoint = Some(Checkpoint {
            sequence: self.checkpoint_sequence,
            waiting: self.writes.keys().copied().collect(),
            relog,
            tail: Vec::new(),
        });
    }

    /// Restarts the log once every write the checkpoint waits for landed.
    fn finish_checkpoint(&mut self) {
        if self
            .checkpoint
            .as_ref()
            .is_none_or(|c| !c.waiting.is_empty())
        {
            return;
        }
        let Some(checkpoint) = self.checkpoint.take() else {
            return;
        };
        // The new log replaces the old one atomically, so a crash or a
        // failure leaves one of the two whole.
        let mut records = Vec::with_capacity(1 + checkpoint.relog.len() + checkpoint.tail.len());
        records.push(WalRecord::Checkpoint(checkpoint.sequence));
        records.extend(checkpoint.relog);
        records.extend(checkpoint.tail);
        match self.wal.replace(&records) {
            Ok(()) => self.checkpoints += 1,
            Err(e) => self.errors.push(format!("edit log checkpoint: {e}")),
        }
        self.wal_bytes = file_len(&self.wal_path);
        self.checkpoint_at = self.wal_bytes + self.checkpoint_size();
    }

    /// Gives up the running checkpoint after a failed write; the log keeps
    /// everything, and the next try waits for it to grow again.
    fn abort_checkpoint(&mut self) {
        if self.checkpoint.take().is_some() {
            self.checkpoint_at = self.wal_bytes + self.checkpoint_size();
        }
    }

    /// Shares the edited chunks' new content with the render tree, and
    /// reports the edited bricks for it.
    fn share_edits(&mut self, chunks: &BTreeSet<ChunkCoord>, bricks: &[BrickPos]) {
        for &c in chunks {
            if let Some(data) = self.tree.chunk_data(c.min_voxel()) {
                let _ = self.render.insert_chunk(c.min_voxel(), data);
            }
        }
        self.changes.render_bricks.extend_from_slice(bricks);
    }

    /// Updates the cached size of loaded chunk `coord` and the hot total.
    fn refresh_bytes(&mut self, coord: ChunkCoord) {
        let bytes = self
            .tree
            .chunk_data(coord.min_voxel())
            .map_or(0, |d| d.bytes());
        if let Some(state) = self.loaded.get_mut(&coord) {
            self.hot_bytes = self.hot_bytes - state.bytes + bytes;
            state.bytes = bytes;
        }
    }

    fn log(&mut self, record: &WalRecord) {
        if let Err(e) = self.wal.append(record) {
            self.errors.push(format!("edit log append: {e}"));
        }
        if let Some(checkpoint) = &mut self.checkpoint {
            checkpoint.tail.push(record.clone());
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
                refused,
                error,
                source,
                micros,
                heights,
                uniform,
            } => {
                self.loading.remove(&coord);
                let n = &mut self.counters;
                match source {
                    LoadSource::Warm => n.warm_hits += 1,
                    LoadSource::Saved => n.disk_loads += 1,
                    LoadSource::Generated => {
                        n.generated += 1;
                        n.generated_micros += micros;
                        n.generated_max_micros = n.generated_max_micros.max(micros);
                        n.uniform_loads += usize::from(uniform);
                    }
                }
                if let Some((lo, hi)) = heights {
                    self.learn_heights(coord.x(), coord.z(), lo, hi, true, eye);
                }
                if let Some(e) = error {
                    self.errors.push(e);
                }
                if refused {
                    self.refused.insert(coord);
                }
                let wanted = eye.x == i32::MIN
                    || plan::chunk_distance(eye, coord) <= plan::voxels(self.config.evict_radius_m)
                    || self.pending.contains_key(&coord);
                if wanted && !self.loaded.contains_key(&coord) {
                    self.ready.push_back((coord, data, false));
                } else if !wanted {
                    // The player moved on before it arrived: keep it warm
                    // for a return.
                    self.keep_warm(coord, data);
                }
            }
            Done::Saved {
                coord,
                warm,
                coarse,
                error,
            } => self.finish_write(coord, warm, coarse, error),
            Done::Coarse {
                x,
                z,
                chunks,
                heights,
            } => {
                self.coarse_loading.remove(&(x, z));
                if let Some((lo, hi)) = heights {
                    self.learn_heights(x, z, lo, hi, false, eye);
                }
                if self.coarse_stale.remove(&(x, z)) {
                    // Built before one of its chunks was written; asked for
                    // again by the next update.
                    return;
                }
                let wanted = eye.x == i32::MIN
                    || plan::column_distance(eye, x, z)
                        <= plan::voxels(self.config.coarse_radius_m) * 1.25;
                if !wanted || self.coarse_loaded.contains_key(&(x, z)) {
                    return;
                }
                let mut entry = Vec::new();
                for (coord, data) in chunks {
                    let bytes = data.bytes();
                    if self
                        .coarse
                        .insert_chunk(coord.min_voxel(), data.clone())
                        .is_ok()
                    {
                        entry.push((coord, bytes));
                        self.coarse_bytes += bytes;
                        self.coarse_chunks.insert(coord, data);
                        self.refresh_render_if_coarse(coord);
                    }
                }
                self.coarse_loaded.insert((x, z), entry);
                self.changes.coarse = true;
            }
            Done::Compressed {
                coord,
                warm,
                ticket,
            } => {
                self.compress_in_flight = self.compress_in_flight.saturating_sub(1);
                if self.compressing.get(&coord) != Some(&ticket) {
                    return;
                }
                self.compressing.remove(&coord);
                let edited = self.saved.contains(&coord);
                self.warm.insert(
                    coord,
                    warm,
                    edited,
                    self.warm_cap(),
                    self.config.warm_edited_weight,
                );
            }
        }
    }

    /// Records the terrain heights of chunk column (`x`, `z`); sampled ones
    /// never replace exact ones. Past [`MAX_HEIGHTS`] the columns farthest
    /// from `eye` are dropped.
    fn learn_heights(&mut self, x: u32, z: u32, lo: i32, hi: i32, exact: bool, eye: VoxelPos) {
        let entry = self
            .heights
            .entry((x, z))
            .or_insert(Heights { lo, hi, exact });
        if exact || !entry.exact {
            *entry = Heights { lo, hi, exact };
        }
        if self.heights.len() > MAX_HEIGHTS && eye.x != i32::MIN {
            let mut far: Vec<(u32, (u32, u32))> = self
                .heights
                .keys()
                .map(|&(x, z)| (plan::column_distance(eye, x, z).to_bits(), (x, z)))
                .collect();
            far.sort_unstable_by(|a, b| b.cmp(a));
            for (_, column) in far.into_iter().take(self.heights.len() - MAX_HEIGHTS / 2) {
                self.heights.remove(&column);
            }
        }
    }

    /// How far `chunk`'s load is put off, in voxels: buried chunks by
    /// `buried_delay_m`, others not at all.
    fn load_delay(&self, chunk: ChunkCoord) -> f32 {
        let heights = self.heights.get(&(chunk.x(), chunk.z())).copied();
        if plan::is_buried(chunk, heights) {
            plan::voxels(self.config.buried_delay_m)
        } else {
            0.0
        }
    }

    fn warm_cap(&self) -> usize {
        self.config.warm_cap_mib as usize * MIB
    }

    /// Loads and compressions in flight at most.
    fn in_flight_cap(&self) -> usize {
        (self.config.workers.max(1) as usize) * 4
    }

    /// Keeps the unchanged chunk `data` at `coord` in the warm cache: its
    /// entry is marked used if it has one (still exact: an entry is replaced
    /// whenever the chunk is written), else it waits for a worker to
    /// compress it.
    fn keep_warm(&mut self, coord: ChunkCoord, data: ChunkData) {
        if self.warm_cap() == 0 || self.warm.touch(coord) {
            return;
        }
        self.tickets += 1;
        self.compressing.insert(coord, self.tickets);
        let bytes = data.bytes();
        self.to_compress
            .push_back((coord, data, self.tickets, bytes));
        self.to_compress_bytes += bytes;
        let bound = self.in_flight_cap() * 4;
        while self.to_compress.len() > bound {
            let Some((c, _, ticket, bytes)) = self.to_compress.pop_front() else {
                break;
            };
            self.to_compress_bytes -= bytes;
            if self.compressing.get(&c) == Some(&ticket) {
                self.compressing.remove(&c);
            }
        }
    }

    /// Hands waiting compressions to the workers while fewer loads than the
    /// in-flight cap are outstanding, the most recently evicted first (the
    /// nearest to the player, so the likeliest to be wanted again).
    fn submit_compressions(&mut self) {
        let cap = self.in_flight_cap();
        while self.loading.len() + self.compress_in_flight < cap {
            let Some((coord, data, ticket, bytes)) = self.to_compress.pop_back() else {
                break;
            };
            self.to_compress_bytes -= bytes;
            if self.compressing.get(&coord) != Some(&ticket) {
                continue;
            }
            self.compress_in_flight += 1;
            self.pool.submit(Request::Compress {
                coord,
                data,
                ticket,
            });
        }
    }

    /// A chunk write landed (or failed).
    fn finish_write(
        &mut self,
        coord: ChunkCoord,
        warm: WarmChunk,
        coarse: Option<ChunkData>,
        error: Option<String>,
    ) {
        let failed = error.is_some();
        match error {
            Some(e) => {
                self.errors.push(format!("saving chunk {coord:?}: {e}"));
                self.write_failures += 1;
                self.abort_checkpoint();
            }
            None => {
                self.saved.insert(coord);
                self.refused.remove(&coord);
                // The region file changed: compressions of older content
                // and the warm copy are stale, even if another write of the
                // chunk is queued behind this one.
                self.compressing.remove(&coord);
                self.warm.remove(coord);
            }
        }
        let Some(w) = self.writes.get_mut(&coord) else {
            return;
        };
        if let Some(next) = w.next.take() {
            self.submit_write(coord, next);
            return;
        }
        w.in_flight = false;
        if failed {
            if let Some(state) = self.loaded.get_mut(&coord) {
                state.edited = true;
            }
            // Evicted content stays held (and drawn) until a retry lands.
            if w.held.is_none() {
                self.writes.remove(&coord);
            }
            return;
        }
        if let Some(w) = self.writes.remove(&coord)
            && let Some((_, bytes)) = w.held
        {
            self.held_bytes -= bytes;
        }
        if let Some(checkpoint) = &mut self.checkpoint {
            checkpoint.waiting.remove(&coord);
        }
        let cap = self.warm_cap();
        self.warm
            .insert(coord, warm, true, cap, self.config.warm_edited_weight);
        if let Some(coarse) = coarse {
            self.replace_coarse(coord, coarse);
        }
        self.refresh_render_if_coarse(coord);
    }

    /// Replaces the coarse chunk at `coord` with the summary of its written
    /// content, so the change shows from afar.
    fn replace_coarse(&mut self, coord: ChunkCoord, data: ChunkData) {
        let column = (coord.x(), coord.z());
        if self.coarse_loading.contains(&column) {
            self.coarse_stale.insert(column);
            return;
        }
        // A column that is not loaded reads the summary from the region
        // file when it is.
        let Some(entry) = self.coarse_loaded.get_mut(&column) else {
            return;
        };
        let bytes = data.bytes();
        if self
            .coarse
            .insert_chunk(coord.min_voxel(), data.clone())
            .is_err()
        {
            return;
        }
        match entry.iter_mut().find(|(c, _)| *c == coord) {
            Some((_, old)) => {
                self.coarse_bytes = self.coarse_bytes - *old + bytes;
                *old = bytes;
            }
            None => {
                entry.push((coord, bytes));
                self.coarse_bytes += bytes;
            }
        }
        self.coarse_chunks.insert(coord, data);
        self.changes.coarse = true;
    }

    /// Refreshes the render tree at `coord` when it shows the coarse chunk
    /// there (the chunk is neither loaded nor held).
    fn refresh_render_if_coarse(&mut self, coord: ChunkCoord) {
        let held = self.writes.get(&coord).is_some_and(|w| w.held.is_some());
        if !self.loaded.contains_key(&coord) && !held {
            self.refresh_render(coord);
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
        let mut state = ChunkState { edited, bytes: 0 };
        for edit in self.pending.remove(&coord).unwrap_or_default() {
            let result = match edit {
                PendingEdit::Edits(edits) => self.tree.apply(&edits).map(|b| !b.is_empty()),
                PendingEdit::Fill(region, m) => self.tree.fill_box(region, m),
            };
            match result {
                // Edits the region file already has change nothing.
                Ok(changed) => state.edited |= changed,
                Err(e) => self.errors.push(format!("waiting edit for {coord:?}: {e}")),
            }
        }
        state.bytes = self.tree.chunk_data(min).map_or(0, |d| d.bytes());
        self.hot_bytes += state.bytes;
        self.loaded.insert(coord, state);
        self.changes.loaded.push(coord);
        self.refresh_render(coord);
    }

    fn request_loads(&mut self, eye: VoxelPos) {
        let in_flight_cap = self.in_flight_cap();
        let sphere = plan::sphere(eye, self.config.full_radius_m);
        self.count_holes(eye, &sphere);
        for coord in self.load_order(eye, sphere) {
            if self.loaded.contains_key(&coord)
                || self.loading.contains(&coord)
                || self.ready.iter().any(|(c, _, _)| *c == coord)
            {
                continue;
            }
            if let Some(w) = self.writes.get_mut(&coord) {
                // Evicted a moment ago and not yet written: rejoin as is.
                // Without held content the chunk is already joining; either
                // way the region file is not read while it is written.
                if let Some((data, bytes)) = w.held.take() {
                    self.held_bytes -= bytes;
                    if !w.in_flight {
                        // A failed write: the content is only here now.
                        self.writes.remove(&coord);
                    }
                    self.ready.push_back((coord, data, true));
                }
                continue;
            }
            if self.loading.len() >= in_flight_cap {
                break;
            }
            self.loading.insert(coord);
            self.pool.submit(Request::Load {
                coord,
                warm: self.warm.get(coord),
                saved: self.saved.contains(&coord),
            });
        }
    }

    /// The order to request chunks in: the full-detail `sphere` (nearest
    /// first, buried chunks put off by `buried_delay_m`) ahead of the motion
    /// first, then, once all of it is requested and while the hot tier is
    /// under its cap, the lookahead sphere. Every chunk of the full-detail
    /// sphere precedes every lookahead chunk, so prefetching never starves
    /// it.
    fn load_order(&self, eye: VoxelPos, sphere: Vec<ChunkCoord>) -> Vec<ChunkCoord> {
        let c = &self.config;
        let predicted = self.motion.predicted(
            eye,
            c.prefetch_lookahead_secs,
            plan::voxels(c.evict_radius_m),
        );
        // A player underground sees the rock around them, not the surface
        // overhead: no chunk is put off then.
        let underground = ChunkCoord::containing(eye).is_some_and(|c| self.load_delay(c) > 0.0);
        let delay = |chunk| {
            if underground {
                0.0
            } else {
                self.load_delay(chunk)
            }
        };
        let mut order = plan::load_priority(eye, predicted, &sphere, delay);
        if let Some(predicted) = predicted
            && self.hot_bytes_in_use() < c.hot_cap_mib as usize * MIB
        {
            // The predicted position is at most evict_radius_m away and
            // only chunks within that of the eye are kept, so a larger
            // radius would only add chunks to sort and drop.
            let extra = plan::lookahead(
                eye,
                predicted,
                c.prefetch_radius_m.min(2.0 * c.evict_radius_m),
                c.full_radius_m,
                c.evict_radius_m,
            );
            order.extend(plan::delayed(predicted, &extra, delay));
        }
        order
    }

    /// Records the chunks of the full-detail `sphere` around `eye` that are
    /// not loaded.
    fn count_holes(&mut self, eye: VoxelPos, sphere: &[ChunkCoord]) {
        let (mut holes, mut ahead) = (0, 0);
        for &c in sphere {
            if !self.loaded.contains_key(&c) {
                holes += 1;
                if self.motion.is_ahead(eye, c) {
                    ahead += 1;
                }
            }
        }
        let n = &mut self.counters;
        n.holes = holes;
        n.holes_ahead = ahead;
        n.hole_sum += holes as u64;
        n.hole_ahead_sum += ahead as u64;
        n.updates += 1;
    }

    /// RAM held by full-detail chunks: the loaded ones, evicted ones
    /// waiting for their writes and unchanged ones waiting to be compressed.
    fn hot_bytes_in_use(&self) -> usize {
        self.hot_bytes + self.held_bytes + self.to_compress_bytes
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
        let mut bytes = self.hot_bytes_in_use();
        if bytes > cap {
            let full = plan::voxels(self.config.full_radius_m);
            let mut extra: Vec<(u32, ChunkCoord, usize)> = self
                .loaded
                .iter()
                .filter(|(c, _)| !doomed.contains(c))
                .map(|(&c, s)| (plan::chunk_distance(eye, c).to_bits(), c, s.bytes))
                .filter(|&(d, _, _)| f32::from_bits(d) > full)
                .collect();
            extra.sort_unstable_by(|a, b| b.cmp(a));
            for (_, c, chunk_bytes) in extra {
                if bytes <= cap {
                    break;
                }
                bytes = bytes.saturating_sub(chunk_bytes);
                doomed.push(c);
            }
        }
        for c in doomed {
            self.unload(c);
        }
        // An unloaded chunk waiting to be compressed still holds its RAM,
        // so unloading alone does not bring the total under the cap: drop
        // the oldest waiting compressions until it fits. Otherwise every
        // frame would evict another batch for the same bytes.
        while self.hot_bytes_in_use() > cap {
            let Some((c, _, ticket, chunk_bytes)) = self.to_compress.pop_front() else {
                break;
            };
            self.to_compress_bytes -= chunk_bytes;
            if self.compressing.get(&c) == Some(&ticket) {
                self.compressing.remove(&c);
            }
        }
    }

    fn unload(&mut self, coord: ChunkCoord) {
        let Some(state) = self.loaded.remove(&coord) else {
            return;
        };
        self.hot_bytes -= state.bytes;
        let Ok(data) = self.tree.take_chunk(coord.min_voxel()) else {
            return;
        };
        if state.edited {
            self.save(coord, data, true);
        } else if let Some(w) = self.writes.get_mut(&coord) {
            // Unchanged since its write was submitted: hold it until that
            // lands, so the region file is not read before then.
            let bytes = data.bytes();
            if let Some((_, old)) = w.held.replace((data, bytes)) {
                self.held_bytes -= old;
            }
            self.held_bytes += bytes;
        } else if !self.refused.contains(&coord) {
            // What the region file or the generator gives: kept compressed,
            // so a return does not generate it again. A refused chunk is
            // not: it stands in for a record that did not load, which the
            // next visit reads again.
            self.keep_warm(coord, data);
        }
        self.changes.unloaded.push(coord);
        self.refresh_render(coord);
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
                    self.coarse_chunks.remove(&coord);
                    self.refresh_render_if_coarse(coord);
                }
                self.changes.coarse = true;
            }
        }
    }
}

#[cfg(test)]
impl StreamedWorld {
    /// The config, to change budgets a test cannot reach otherwise.
    pub(super) fn config_mut(&mut self) -> &mut ResidencyConfig {
        &mut self.config
    }

    /// The bytes of the loaded chunks, summed afresh from the tree.
    pub(super) fn hot_bytes_walked(&self) -> usize {
        self.loaded
            .keys()
            .map(|c| self.tree.chunk_data(c.min_voxel()).map_or(0, |d| d.bytes()))
            .sum()
    }

    /// RAM the hot cap is checked against.
    pub(super) fn hot_bytes_counted(&self) -> usize {
        self.hot_bytes_in_use()
    }

    /// The warm cache's bytes, summed afresh from its entries.
    pub(super) fn warm_bytes_walked(&self) -> usize {
        self.warm.bytes_walked()
    }

    /// Whether the warm cache holds `coord`.
    pub(super) fn is_warm(&self, coord: ChunkCoord) -> bool {
        self.warm.contains(coord)
    }

    /// The loaded content of `coord`.
    pub(super) fn chunk_data(&self, coord: ChunkCoord) -> Option<ChunkData> {
        self.loaded
            .contains_key(&coord)
            .then(|| self.tree.chunk_data(coord.min_voxel()))
            .flatten()
    }

    /// The loaded chunks.
    pub(super) fn column_cache(&self) -> &ColumnCache {
        &self.columns
    }

    pub(super) fn loaded_chunks(&self) -> Vec<ChunkCoord> {
        self.loaded.keys().copied().collect()
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
