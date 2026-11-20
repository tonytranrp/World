//! The work the residency threads do, as plain functions: loading a chunk
//! (generated, from its saved overlay or full record, or from the warm
//! cache), encoding an edited chunk for saving, and building a coarse chunk
//! column. Each builds a standalone tree and hands back detached chunk data,
//! which joins the world in O(1).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, OnceLock};

use world_core::serialize::codec::{self, Codec};
use world_core::serialize::region::RecordKind;
use world_core::serialize::{decode, encode};
use world_core::tree::{CHUNK_LEVEL, ChunkData};
use world_core::{MaterialId, VoxelTree};
use world_gen::{CHUNK_SIZE, ChunkCoord, CoarseSource, Generator, GeneratorVersion};

use super::store::SavedChunk;

/// A chunk kept compressed in RAM after eviction: the LZ4 of the full
/// encoding of a tree holding only it. The bytes are shared, so handing an
/// entry to a load copies nothing.
#[derive(Clone, Debug)]
pub struct WarmChunk {
    pub bytes: Arc<[u8]>,
    pub len: usize,
}

impl WarmChunk {
    /// Bytes held.
    pub fn size(&self) -> usize {
        self.bytes.len()
    }
}

/// The world's generator and every older version this build supports, for
/// the same seed and terrain. An overlay saved against an older version is
/// merged onto that version's base, so its edits land where they were made.
pub struct Generators {
    current: Arc<Generator>,
    /// Older versions, each built the first time a record asks for it.
    older: BTreeMap<u32, OnceLock<Option<Generator>>>,
}

impl Generators {
    pub fn new(current: Arc<Generator>) -> Self {
        let now = current.id().version;
        let older = GeneratorVersion::SUPPORTED
            .iter()
            .filter(|&&v| v != now)
            .map(|v| (v.0, OnceLock::new()))
            .collect();
        Self { current, older }
    }

    /// The generator new chunks come from.
    pub fn current(&self) -> &Generator {
        &self.current
    }

    /// The version of [`current`](Self::current), stored in every record
    /// written.
    pub fn version(&self) -> u32 {
        self.current.id().version.0
    }

    /// The generator of `version` with the current seed and terrain, or
    /// `None` when this build cannot generate it.
    pub fn get(&self, version: u32) -> Option<&Generator> {
        if version == self.version() {
            return Some(&self.current);
        }
        self.older
            .get(&version)?
            .get_or_init(|| {
                Generator::new(
                    self.current.id().seed,
                    GeneratorVersion(version),
                    self.current.config().clone(),
                )
                .ok()
            })
            .as_ref()
    }
}

/// Where a loaded chunk's content came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadSource {
    /// Decompressed from the warm cache.
    Warm,
    /// Decoded from its region file records.
    Saved,
    /// Generated from the seed.
    Generated,
}

/// A loaded chunk, ready to join the world. Its content is what the warm
/// cache, the region file or the generator holds, so it needs no saving
/// until it changes.
pub struct Loaded {
    pub data: ChunkData,
    /// A saved record could not be used (unreadable, or made by a generator
    /// version this build does not have), so the chunk came from another
    /// record or the generator. The record stays on disk and is set aside
    /// before anything replaces it.
    pub refused: bool,
    pub error: Option<String>,
    pub source: LoadSource,
}

/// Generates the chunk at `coord` into detached data.
pub fn generate(generator: &Generator, coord: ChunkCoord) -> ChunkData {
    let source = generator.chunk_source(coord);
    let mut tree = VoxelTree::new();
    let min = coord.min_voxel();
    // The coordinate names a chunk inside the world, so both calls succeed.
    let _ = tree.build_region(CHUNK_LEVEL, min, &source);
    tree.take_chunk(min).unwrap_or_else(|_| empty())
}

/// Detached all-air content, for impossible fallbacks.
fn empty() -> ChunkData {
    let mut tree = VoxelTree::new();
    tree.take_chunk(world_core::VoxelPos::new(0, 0, 0))
        .expect("the chunk at the world centre exists and is resident")
}

/// Loads `coord` from a warm entry, saved records or the generator, in that
/// order of preference. An overlay is merged onto the base of the generator
/// version it was saved with; one from a version this build cannot generate
/// is refused (and kept on disk). A full record does not depend on the
/// generator.
pub fn load(
    generators: &Generators,
    coord: ChunkCoord,
    warm: Option<&WarmChunk>,
    saved: &[SavedChunk],
) -> Loaded {
    let min = coord.min_voxel();
    let mut error = None;
    let mut refused = false;
    if let Some(warm) = warm {
        match codec::decompress(Codec::Lz4, &warm.bytes, warm.len)
            .map_err(|e| e.to_string())
            .and_then(|bytes| decode(&bytes).map_err(|e| e.to_string()))
        {
            Ok(mut tree) => {
                if let Ok(data) = tree.take_chunk(min) {
                    return Loaded {
                        data,
                        refused: false,
                        error: None,
                        source: LoadSource::Warm,
                    };
                }
            }
            Err(e) => {
                // The warm copy stands for the region file, which was not
                // read: refuse it, so that file is set aside before anything
                // replaces it.
                error = Some(format!("warm chunk {coord:?}: {e}"));
                refused = true;
            }
        }
    }
    for record in saved {
        let base = match record.kind {
            RecordKind::Overlay => match generators.get(record.generator) {
                Some(generator) => Some(generator),
                None => {
                    error = Some(format!(
                        "saved chunk {coord:?}: an overlay made with generator version {}, which \
                         this build cannot generate",
                        record.generator
                    ));
                    refused = true;
                    continue;
                }
            },
            _ => None,
        };
        let result = decode(&record.bytes)
            .map_err(|e| e.to_string())
            .and_then(|saved_tree| {
                let mut tree = match (record.kind, base) {
                    (RecordKind::Full, _) => saved_tree,
                    (RecordKind::Overlay, Some(generator)) => {
                        let base = generator.chunk_source(coord);
                        let mut tree = VoxelTree::unloaded();
                        tree.apply_overlay(min, &base, &saved_tree)
                            .map_err(|e| e.to_string())?;
                        tree
                    }
                    _ => return Err("this record cannot load a chunk".into()),
                };
                tree.take_chunk(min).map_err(|e| e.to_string())
            });
        match result {
            Ok(data) => {
                return Loaded {
                    data,
                    refused,
                    error,
                    source: LoadSource::Saved,
                };
            }
            Err(e) => {
                error = Some(format!("saved chunk {coord:?} ({:?}): {e}", record.kind));
                refused = true;
            }
        }
    }
    Loaded {
        data: generate(generators.current(), coord),
        refused,
        error,
        source: LoadSource::Generated,
    }
}

/// What saving an edited chunk produced.
pub struct Encoded {
    /// The records to write: the overlay or the full content, and the coarse
    /// summary.
    pub records: Vec<SavedChunk>,
    pub warm: WarmChunk,
    /// The coarse summary as chunk data, which replaces the coarse chunk the
    /// world shows from afar.
    pub coarse: Option<ChunkData>,
}

/// Encodes the edited chunk `data` at `coord`: its overlay against the base
/// of the current generator, or its full content when the overlay is more
/// than twice as large, plus a coarse summary with `4^coarse_level` cells.
pub fn encode_chunk(
    generators: &Generators,
    coord: ChunkCoord,
    data: ChunkData,
    coarse_level: u32,
) -> Encoded {
    let min = coord.min_voxel();
    let mut tree = VoxelTree::unloaded();
    // An aligned chunk position always inserts.
    let _ = tree.insert_chunk(min, data);
    let full = encode(&tree);
    let generator_version = generators.version();
    let base = generators.current().chunk_source(coord);
    let overlay = tree.overlay(min, &base).map(|o| encode(&o)).ok();
    let primary = match overlay {
        Some(bytes) if bytes.len() <= 2 * full.len() => SavedChunk {
            kind: RecordKind::Overlay,
            generator: generator_version,
            bytes,
        },
        _ => SavedChunk {
            kind: RecordKind::Full,
            generator: generator_version,
            bytes: full.clone(),
        },
    };
    let mut records = vec![primary];
    let mut coarse_data = None;
    if let Ok(mut coarse) = tree.extract_coarse(CHUNK_LEVEL, min, coarse_level) {
        records.push(SavedChunk {
            kind: RecordKind::Coarse(coarse_level as u8),
            generator: generator_version,
            bytes: encode(&coarse),
        });
        coarse_data = coarse.take_chunk(min).ok();
    }
    let warm = warm_of(&full);
    Encoded {
        records,
        warm,
        coarse: coarse_data,
    }
}

/// The warm form of a chunk's full encoding.
fn warm_of(full: &[u8]) -> WarmChunk {
    WarmChunk {
        bytes: codec::compress(Codec::Lz4, full).into(),
        len: full.len(),
    }
}

/// Compresses the unchanged chunk `data` at `coord` for the warm cache.
pub fn compress(coord: ChunkCoord, data: ChunkData) -> WarmChunk {
    let mut tree = VoxelTree::unloaded();
    // An aligned chunk position always inserts.
    let _ = tree.insert_chunk(coord.min_voxel(), data);
    warm_of(&encode(&tree))
}

/// The chunks of coarse column (`x`, `z`) that cross the terrain surface,
/// one below it included, built with `4^level` cells. `saved` holds coarse
/// summaries of edited chunks, which replace the generated ones wherever
/// they are (a tower built far above the surface shows too).
pub fn coarse_column(
    generator: &Generator,
    x: u32,
    z: u32,
    level: u32,
    saved: &[(ChunkCoord, SavedChunk)],
) -> Vec<(ChunkCoord, ChunkData)> {
    let Ok(probe) = ChunkCoord::new(x, 0, z) else {
        return Vec::new();
    };
    let Ok(columns) = generator.coarse_columns(probe, level) else {
        return Vec::new();
    };
    let (lo, hi) = columns
        .columns()
        .iter()
        .fold((i32::MAX, i32::MIN), |(lo, hi), c| {
            (lo.min(c.height), hi.max(c.height))
        });
    let min_voxel = probe.min_voxel();
    let chunk_of =
        |y: i32| ChunkCoord::containing(world_core::VoxelPos::new(min_voxel.x, y, min_voxel.z));
    let (Some(bottom), Some(top)) = (chunk_of(lo.saturating_sub(CHUNK_SIZE as i32)), chunk_of(hi))
    else {
        return Vec::new();
    };
    let mut ys: BTreeSet<u32> = (bottom.y()..=top.y()).collect();
    ys.extend(
        saved
            .iter()
            .filter(|(c, _)| c.x() == x && c.z() == z)
            .map(|(c, _)| c.y()),
    );
    let mut out = Vec::new();
    for y in ys {
        let Ok(coord) = ChunkCoord::new(x, y, z) else {
            continue;
        };
        let min = coord.min_voxel();
        let from_save = saved
            .iter()
            .find(|(c, _)| *c == coord)
            .and_then(|(_, s)| decode(&s.bytes).ok())
            .and_then(|mut t| t.take_chunk(min).ok());
        let data = from_save.unwrap_or_else(|| {
            // Coarse columns depend only on x and z, so they serve every y.
            let source = CoarseSource::new(columns.clone());
            let mut tree = VoxelTree::new();
            let _ = tree.build_region(CHUNK_LEVEL, min, &source);
            tree.take_chunk(min).unwrap_or_else(|_| empty())
        });
        if data.uniform() != Some(MaterialId::AIR) {
            out.push((coord, data));
        }
    }
    out
}
