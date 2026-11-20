//! The residency threads: one I/O thread that alone touches region files,
//! and worker threads that generate, decode and encode chunks. The main
//! thread submits requests and drains results once per frame; it never
//! waits except in [`Pool::wait_idle`].
//!
//! With zero workers every request runs at once on the calling thread, which
//! keeps tests deterministic.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use world_core::serialize::region::RecordKind;
use world_core::tree::ChunkData;
use world_gen::ChunkCoord;

use super::jobs::{self, Generators, LoadSource, WarmChunk};
use super::store::{RegionStore, SavedChunk};

/// What the main thread asks for.
pub enum Request {
    /// Load a chunk: from `warm` if given, else from its saved records if
    /// `saved`, else generate it.
    Load {
        coord: ChunkCoord,
        warm: Option<WarmChunk>,
        saved: bool,
    },
    /// Encode an edited chunk and write it to its region file. With
    /// `set_aside`, the chunk's current records are copied aside first (one
    /// of them could not be loaded), and nothing is written if that fails.
    Save {
        coord: ChunkCoord,
        data: ChunkData,
        set_aside: bool,
    },
    /// Build a coarse chunk column; `edited` lists its chunks that have
    /// saved coarse summaries.
    Coarse {
        x: u32,
        z: u32,
        edited: Vec<ChunkCoord>,
    },
}

/// What comes back.
pub enum Done {
    Loaded {
        coord: ChunkCoord,
        data: ChunkData,
        refused: bool,
        error: Option<String>,
        source: LoadSource,
    },
    Saved {
        coord: ChunkCoord,
        warm: WarmChunk,
        /// The new coarse summary of the chunk.
        coarse: Option<ChunkData>,
        error: Option<String>,
    },
    Coarse {
        x: u32,
        z: u32,
        chunks: Vec<(ChunkCoord, ChunkData)>,
    },
}

/// Settings shared by every thread.
#[derive(Clone)]
pub struct Shared {
    pub generators: Arc<Generators>,
    pub coarse_level: u32,
}

/// An encoded chunk on its way to the I/O thread.
struct Write {
    coord: ChunkCoord,
    records: Vec<SavedChunk>,
    warm: WarmChunk,
    coarse: Option<ChunkData>,
    set_aside: bool,
}

enum IoJob {
    /// Ends the I/O thread, which then releases its handle on the workers'
    /// queue so they can end too.
    Stop,
    ReadForLoad(ChunkCoord),
    ReadForCoarse(u32, u32, Vec<ChunkCoord>),
    Write(Write),
}

enum WorkJob {
    Load(ChunkCoord, Option<WarmChunk>, Vec<SavedChunk>),
    Save(ChunkCoord, ChunkData, bool),
    Coarse(u32, u32, Vec<(ChunkCoord, SavedChunk)>),
}

/// The I/O thread and the workers.
pub struct Pool {
    shared: Shared,
    mode: Mode,
    outstanding: usize,
}

enum Mode {
    /// Everything runs on the calling thread; results wait here.
    Inline { store: RegionStore, done: Vec<Done> },
    Threads {
        io: Option<Sender<IoJob>>,
        work: Option<Sender<WorkJob>>,
        /// Behind a lock only so the pool can live in a shared resource; the
        /// main thread is the one reader.
        done: Mutex<Receiver<Done>>,
        threads: Vec<JoinHandle<()>>,
    },
}

impl Pool {
    /// Starts `workers` worker threads and the I/O thread, or none at all
    /// when `workers` is 0.
    pub fn new(shared: Shared, store: RegionStore, workers: u32) -> Self {
        if workers == 0 {
            return Self {
                shared,
                mode: Mode::Inline {
                    store,
                    done: Vec::new(),
                },
                outstanding: 0,
            };
        }
        let (done_tx, done_rx) = mpsc::channel::<Done>();
        let (work_tx, work_rx) = mpsc::channel::<WorkJob>();
        let (io_tx, io_rx) = mpsc::channel::<IoJob>();
        let work_rx = Arc::new(Mutex::new(work_rx));
        let mut threads = Vec::new();

        {
            let work_tx = work_tx.clone();
            let done_tx = done_tx.clone();
            let level = shared.coarse_level;
            let spawned = std::thread::Builder::new()
                .name("world-io".into())
                .spawn(move || io_loop(&store, level, &io_rx, &work_tx, &done_tx));
            if let Ok(handle) = spawned {
                threads.push(handle);
            }
        }
        for i in 0..workers {
            let work_rx = Arc::clone(&work_rx);
            let io_tx = io_tx.clone();
            let done_tx = done_tx.clone();
            let shared = shared.clone();
            let spawned = std::thread::Builder::new()
                .name(format!("world-worker-{i}"))
                .spawn(move || work_loop(&shared, &work_rx, &io_tx, &done_tx));
            if let Ok(handle) = spawned {
                threads.push(handle);
            }
        }
        Self {
            shared,
            mode: Mode::Threads {
                io: Some(io_tx),
                work: Some(work_tx),
                done: Mutex::new(done_rx),
                threads,
            },
            outstanding: 0,
        }
    }

    pub fn submit(&mut self, request: Request) {
        self.outstanding += 1;
        let shared = self.shared.clone();
        match &mut self.mode {
            Mode::Inline { store, done } => {
                let job = match request {
                    Request::Load { coord, warm, saved } => {
                        let records = if saved && warm.is_none() {
                            read_saved(store, coord)
                        } else {
                            Vec::new()
                        };
                        WorkJob::Load(coord, warm, records)
                    }
                    Request::Save {
                        coord,
                        data,
                        set_aside,
                    } => WorkJob::Save(coord, data, set_aside),
                    Request::Coarse { x, z, edited } => {
                        WorkJob::Coarse(x, z, read_coarse(store, &edited, shared.coarse_level))
                    }
                };
                match run(&shared, job) {
                    Ran::Done(d) => done.push(d),
                    Ran::Write(w) => done.push(write(store, w)),
                }
            }
            Mode::Threads { io, work, .. } => {
                let (Some(io), Some(work)) = (io.as_ref(), work.as_ref()) else {
                    return;
                };
                // A closed channel means a thread died; the request is
                // dropped and `outstanding` stays raised, so `wait_idle`
                // reports it.
                let _ = match request {
                    Request::Load { coord, warm, saved } if saved && warm.is_none() => {
                        io.send(IoJob::ReadForLoad(coord)).is_ok()
                    }
                    Request::Load { coord, warm, .. } => {
                        work.send(WorkJob::Load(coord, warm, Vec::new())).is_ok()
                    }
                    Request::Save {
                        coord,
                        data,
                        set_aside,
                    } => work.send(WorkJob::Save(coord, data, set_aside)).is_ok(),
                    Request::Coarse { x, z, edited } if !edited.is_empty() => {
                        io.send(IoJob::ReadForCoarse(x, z, edited)).is_ok()
                    }
                    Request::Coarse { x, z, .. } => {
                        work.send(WorkJob::Coarse(x, z, Vec::new())).is_ok()
                    }
                };
            }
        }
    }

    /// Every result ready now.
    pub fn drain(&mut self) -> Vec<Done> {
        let done: Vec<Done> = match &mut self.mode {
            Mode::Inline { done, .. } => std::mem::take(done),
            Mode::Threads { done, .. } => match done.get_mut() {
                Ok(rx) => rx.try_iter().collect(),
                Err(_) => Vec::new(),
            },
        };
        self.outstanding -= done.len().min(self.outstanding);
        done
    }

    /// Blocks until every submitted request has a result, and returns them.
    /// Returns early, with what arrived, if a thread died.
    pub fn wait_idle(&mut self) -> Vec<Done> {
        let mut all = self.drain();
        while self.outstanding > 0 {
            let Mode::Threads { done, .. } = &mut self.mode else {
                break;
            };
            let Ok(rx) = done.get_mut() else {
                break;
            };
            match rx.recv() {
                Ok(d) => {
                    all.push(d);
                    self.outstanding -= 1;
                }
                Err(_) => break,
            }
        }
        all
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        if let Mode::Threads {
            io, work, threads, ..
        } = &mut self.mode
        {
            // The I/O thread stops after the work queued before it; then the
            // last senders of the work queue are gone and the workers end.
            if let Some(io) = io.take() {
                let _ = io.send(IoJob::Stop);
            }
            work.take();
            for t in threads.drain(..) {
                let _ = t.join();
            }
        }
    }
}

fn read_saved(store: &RegionStore, coord: ChunkCoord) -> Vec<SavedChunk> {
    store
        .read(coord, &[RecordKind::Full, RecordKind::Overlay])
        .unwrap_or_default()
}

fn read_coarse(
    store: &RegionStore,
    edited: &[ChunkCoord],
    level: u32,
) -> Vec<(ChunkCoord, SavedChunk)> {
    edited
        .iter()
        .filter_map(|&c| {
            let mut found = store.read(c, &[RecordKind::Coarse(level as u8)]).ok()?;
            Some((c, found.pop()?))
        })
        .collect()
}

fn write(store: &RegionStore, w: Write) -> Done {
    let set_aside = if w.set_aside {
        store.set_aside(w.coord).map(|_| ())
    } else {
        Ok(())
    };
    let error = set_aside
        .and_then(|()| store.write(w.coord, w.records))
        .err()
        .map(|e| e.to_string());
    Done::Saved {
        coord: w.coord,
        warm: w.warm,
        coarse: w.coarse,
        error,
    }
}

enum Ran {
    Done(Done),
    Write(Write),
}

fn run(shared: &Shared, job: WorkJob) -> Ran {
    match job {
        WorkJob::Load(coord, warm, saved) => {
            let loaded = jobs::load(&shared.generators, coord, warm.as_ref(), &saved);
            Ran::Done(Done::Loaded {
                coord,
                data: loaded.data,
                refused: loaded.refused,
                error: loaded.error,
                source: loaded.source,
            })
        }
        WorkJob::Save(coord, data, set_aside) => {
            let e = jobs::encode_chunk(&shared.generators, coord, data, shared.coarse_level);
            Ran::Write(Write {
                coord,
                records: e.records,
                warm: e.warm,
                coarse: e.coarse,
                set_aside,
            })
        }
        WorkJob::Coarse(x, z, saved) => Ran::Done(Done::Coarse {
            x,
            z,
            chunks: jobs::coarse_column(
                shared.generators.current(),
                x,
                z,
                shared.coarse_level,
                &saved,
            ),
        }),
    }
}

fn io_loop(
    store: &RegionStore,
    coarse_level: u32,
    jobs: &Receiver<IoJob>,
    work: &Sender<WorkJob>,
    done: &Sender<Done>,
) {
    for job in jobs {
        match job {
            IoJob::Stop => return,
            IoJob::ReadForLoad(coord) => {
                let _ = work.send(WorkJob::Load(coord, None, read_saved(store, coord)));
            }
            IoJob::ReadForCoarse(x, z, edited) => {
                let saved = read_coarse(store, &edited, coarse_level);
                let _ = work.send(WorkJob::Coarse(x, z, saved));
            }
            IoJob::Write(w) => {
                let _ = done.send(write(store, w));
            }
        }
    }
}

fn work_loop(
    shared: &Shared,
    jobs: &Arc<Mutex<Receiver<WorkJob>>>,
    io: &Sender<IoJob>,
    done: &Sender<Done>,
) {
    loop {
        let job = {
            let Ok(queue) = jobs.lock() else {
                return;
            };
            match queue.recv() {
                Ok(job) => job,
                Err(_) => return,
            }
        };
        match run(shared, job) {
            Ran::Done(d) => {
                if done.send(d).is_err() {
                    return;
                }
            }
            Ran::Write(w) => {
                if io.send(IoJob::Write(w)).is_err() {
                    return;
                }
            }
        }
    }
}
