//! The work the residency threads do, as plain functions: loading a chunk
//! (generated, from its saved overlay or full record, or from the warm
//! cache), encoding an edited chunk for saving, and building a coarse chunk
//! column. Each builds a standalone tree and hands back detached chunk data,
//! which joins the world in O(1).

use world_core::serialize::codec::{self, Codec};
use world_core::serialize::region::RecordKind;
use world_core::serialize::{decode, encode};
use world_core::tree::{CHUNK_LEVEL, ChunkData};
use world_core::{MaterialId, VoxelTree};
use world_gen::{CHUNK_SIZE, ChunkCoord, CoarseSource, Generator};

use super::store::SavedChunk;

/// An edited chunk kept compressed in RAM after eviction: the LZ4 of its
/// full encoding.
#[derive(Clone, Debug)]
pub struct WarmChunk {
    pub bytes: Vec<u8>,
    pub len: usize,
}

impl WarmChunk {
    /// Bytes held.
    pub fn size(&self) -> usize {
        self.bytes.len()
    }
}

/// A loaded chunk, ready to join the world.
pub struct Loaded {
    pub data: ChunkData,
    /// Whether the content differs from the generated terrain.
    pub edited: bool,
    /// A saved record that could not be read; the chunk was generated
    /// instead. The record stays on disk.
    pub error: Option<String>,
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
/// order of preference. An overlay saved by another generator version would
/// land on the wrong base, so it is refused (and kept on disk); a full record
/// does not depend on the generator.
pub fn load(
    generator: &Generator,
    generator_version: u32,
    coord: ChunkCoord,
    warm: Option<&WarmChunk>,
    saved: &[SavedChunk],
) -> Loaded {
    let min = coord.min_voxel();
    let mut error = None;
    if let Some(warm) = warm {
        match codec::decompress(Codec::Lz4, &warm.bytes, warm.len)
            .map_err(|e| e.to_string())
            .and_then(|bytes| decode(&bytes).map_err(|e| e.to_string()))
        {
            Ok(mut tree) => {
                if let Ok(data) = tree.take_chunk(min) {
                    return Loaded {
                        data,
                        edited: true,
                        error: None,
                    };
                }
            }
            Err(e) => error = Some(format!("warm chunk {coord:?}: {e}")),
        }
    }
    for record in saved {
        if record.kind == RecordKind::Overlay && record.generator != generator_version {
            error = Some(format!(
                "saved chunk {coord:?}: an overlay made with generator version {}, this build                  generates version {generator_version}",
                record.generator
            ));
            continue;
        }
        let result = decode(&record.bytes)
            .map_err(|e| e.to_string())
            .and_then(|saved_tree| {
                let mut tree = match record.kind {
                    RecordKind::Full => saved_tree,
                    RecordKind::Overlay => {
                        let base = generator.chunk_source(coord);
                        let mut tree = VoxelTree::unloaded();
                        tree.apply_overlay(min, &base, &saved_tree)
                            .map_err(|e| e.to_string())?;
                        tree
                    }
                    RecordKind::Coarse(_) => {
                        return Err("a coarse record cannot load a chunk".into());
                    }
                };
                tree.take_chunk(min).map_err(|e| e.to_string())
            });
        match result {
            Ok(data) => {
                return Loaded {
                    data,
                    edited: true,
                    error,
                };
            }
            Err(e) => error = Some(format!("saved chunk {coord:?} ({:?}): {e}", record.kind)),
        }
    }
    Loaded {
        data: generate(generator, coord),
        edited: false,
        error,
    }
}

/// What saving an edited chunk produced.
pub struct Encoded {
    /// The records to write: the overlay or the full content, and the coarse
    /// summary.
    pub records: Vec<SavedChunk>,
    pub warm: WarmChunk,
}

/// Encodes the edited chunk `data` at `coord`: its overlay against the
/// generated base, or its full content when that is not more than twice as
/// large, plus a coarse summary with `4^coarse_level` cells.
pub fn encode_chunk(
    generator: &Generator,
    generator_version: u32,
    coord: ChunkCoord,
    data: ChunkData,
    coarse_level: u32,
) -> Encoded {
    let min = coord.min_voxel();
    let mut tree = VoxelTree::unloaded();
    // An aligned chunk position always inserts.
    let _ = tree.insert_chunk(min, data);
    let full = encode(&tree);
    let base = generator.chunk_source(coord);
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
    if let Ok(coarse) = tree.extract_coarse(CHUNK_LEVEL, min, coarse_level) {
        records.push(SavedChunk {
            kind: RecordKind::Coarse(coarse_level as u8),
            generator: generator_version,
            bytes: encode(&coarse),
        });
    }
    let warm = WarmChunk {
        bytes: codec::compress(Codec::Lz4, &full),
        len: full.len(),
    };
    Encoded { records, warm }
}

/// The chunks of coarse column (`x`, `z`) that cross the terrain surface,
/// one below it included, built with `4^level` cells. `saved` holds coarse
/// summaries of edited chunks, which replace the generated ones.
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
    let mut out = Vec::new();
    for y in bottom.y()..=top.y() {
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
