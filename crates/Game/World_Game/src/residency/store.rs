//! The save directory: region files of edited chunks and the edit log.
//!
//! Only the I/O thread touches region files, one request at a time, so a
//! file is never read while it is being replaced (Windows refuses to rename
//! over a file another handle has open).

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use world_core::serialize::codec::Codec;
use world_core::serialize::region::{
    ChunkRecord, REGION_CHUNKS, RecordKind, RegionError, RegionFile, encode_region, write_atomic,
};
use world_gen::ChunkCoord;

/// What a saved chunk holds, decoded from its region file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedChunk {
    pub kind: RecordKind,
    /// The generator version the base came from.
    pub generator: u32,
    /// The encoded tree.
    pub bytes: Vec<u8>,
}

/// Region files under one directory.
pub struct RegionStore {
    dir: PathBuf,
}

/// The region holding `chunk`, and the chunk's place inside it.
fn split(chunk: ChunkCoord) -> ([u32; 3], [u32; 3]) {
    let c = chunk.to_array();
    (c.map(|v| v / REGION_CHUNKS), c.map(|v| v % REGION_CHUNKS))
}

impl RegionStore {
    /// A store in `dir`, which is created if needed.
    pub fn open(dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        Ok(Self {
            dir: dir.to_path_buf(),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, region: [u32; 3]) -> PathBuf {
        self.dir
            .join(format!("r.{}.{}.{}.wvr", region[0], region[1], region[2]))
    }

    /// The saved records of `chunk` of the given kinds, in the order asked.
    /// A missing region file means nothing is saved there.
    pub fn read(
        &self,
        chunk: ChunkCoord,
        kinds: &[RecordKind],
    ) -> Result<Vec<SavedChunk>, RegionError> {
        let (region, inner) = split(chunk);
        let path = self.path(region);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let mut file = RegionFile::open(&path)?;
        let mut out = Vec::new();
        for &kind in kinds {
            let Some(entry) = file.index().find(inner, kind).copied() else {
                continue;
            };
            if let Some(bytes) = file.read(inner, kind)? {
                out.push(SavedChunk {
                    kind,
                    generator: entry.generator,
                    bytes,
                });
            }
        }
        Ok(out)
    }

    /// Replaces every record of `chunk` with `records`, keeping the other
    /// chunks of its region. An empty list removes the chunk.
    pub fn write(&self, chunk: ChunkCoord, records: Vec<SavedChunk>) -> Result<(), RegionError> {
        let (region, inner) = split(chunk);
        let path = self.path(region);
        let mut all: BTreeMap<([u32; 3], RecordKind), ChunkRecord> = BTreeMap::new();
        if path.exists() {
            let mut file = RegionFile::open(&path)?;
            let entries = file.index().entries.clone();
            for e in entries {
                if e.chunk == inner {
                    continue;
                }
                if let Some(bytes) = file.read(e.chunk, e.kind)? {
                    all.insert(
                        (e.chunk, e.kind),
                        ChunkRecord {
                            chunk: e.chunk,
                            kind: e.kind,
                            generator: e.generator,
                            bytes,
                        },
                    );
                }
            }
        }
        for r in records {
            all.insert(
                (inner, r.kind),
                ChunkRecord {
                    chunk: inner,
                    kind: r.kind,
                    generator: r.generator,
                    bytes: r.bytes,
                },
            );
        }
        if all.is_empty() {
            return match fs::remove_file(&path) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e.into()),
                _ => Ok(()),
            };
        }
        let records: Vec<ChunkRecord> = all.into_values().collect();
        write_atomic(&path, &encode_region(region, &records, Codec::Zstd))?;
        Ok(())
    }

    /// Copies every record of `chunk` into the `set_aside` folder, one file
    /// per record named after the chunk, kind and generator version, never
    /// replacing an earlier copy. Used before a chunk whose record could not
    /// be loaded is written, so that record survives for a build that can
    /// read it. Returns how many records were copied.
    pub fn set_aside(&self, chunk: ChunkCoord) -> Result<usize, RegionError> {
        let kinds: Vec<RecordKind> = {
            let (region, inner) = split(chunk);
            let path = self.path(region);
            if !path.exists() {
                return Ok(0);
            }
            let file = RegionFile::open(&path)?;
            file.index()
                .entries
                .iter()
                .filter(|e| e.chunk == inner)
                .map(|e| e.kind)
                .collect()
        };
        let records = self.read(chunk, &kinds)?;
        if records.is_empty() {
            return Ok(0);
        }
        let dir = self.set_aside_dir();
        fs::create_dir_all(&dir)?;
        let [x, y, z] = chunk.to_array();
        for r in &records {
            let kind = match r.kind {
                RecordKind::Overlay => "overlay".to_string(),
                RecordKind::Full => "full".to_string(),
                RecordKind::Coarse(level) => format!("coarse{level}"),
            };
            let stem = format!("c.{x}.{y}.{z}.{kind}.v{}", r.generator);
            let mut n = 0;
            let path = loop {
                let path = dir.join(format!("{stem}.{n}.tree"));
                if !path.exists() {
                    break path;
                }
                n += 1;
            };
            write_atomic(&path, &r.bytes)?;
        }
        Ok(records.len())
    }

    /// Where [`set_aside`](Self::set_aside) puts records.
    pub fn set_aside_dir(&self) -> PathBuf {
        self.dir.join("set_aside")
    }

    /// The edit log's path.
    pub fn wal_path(&self) -> PathBuf {
        self.dir.join("edits.wvl")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("world_store_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn chunks_of_one_region_are_saved_and_replaced_independently() {
        let dir = temp("regions");
        let store = RegionStore::open(&dir).unwrap();
        let a = ChunkCoord::new(32, 33, 34).unwrap();
        let b = ChunkCoord::new(33, 33, 34).unwrap();
        let rec = |kind, byte: u8| SavedChunk {
            kind,
            generator: 1,
            bytes: vec![byte; 300],
        };
        store
            .write(
                a,
                vec![rec(RecordKind::Overlay, 1), rec(RecordKind::Coarse(2), 2)],
            )
            .unwrap();
        store.write(b, vec![rec(RecordKind::Full, 3)]).unwrap();
        assert_eq!(
            store
                .read(a, &[RecordKind::Overlay, RecordKind::Full])
                .unwrap(),
            vec![rec(RecordKind::Overlay, 1)]
        );
        store.write(a, vec![rec(RecordKind::Full, 9)]).unwrap();
        assert_eq!(
            store
                .read(
                    a,
                    &[RecordKind::Overlay, RecordKind::Full, RecordKind::Coarse(2)]
                )
                .unwrap(),
            vec![rec(RecordKind::Full, 9)]
        );
        assert_eq!(
            store.read(b, &[RecordKind::Full]).unwrap(),
            vec![rec(RecordKind::Full, 3)]
        );
        store.write(a, Vec::new()).unwrap();
        store.write(b, Vec::new()).unwrap();
        assert!(store.read(b, &[RecordKind::Full]).unwrap().is_empty());
        assert_eq!(
            fs::read_dir(&dir).unwrap().count(),
            0,
            "empty regions are removed"
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
