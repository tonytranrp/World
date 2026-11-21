//! The write-ahead edit log (decision 0006): every edit batch is appended
//! before the frame that made it ends, so a crash loses nothing that region
//! files have not yet caught up with.
//!
//! # Format, version 1
//!
//! A 8-byte header, [`WAL_MAGIC`] then the version `u16` and two zero bytes,
//! followed by records. A record is its payload length `u32`, the xxh3-64 of
//! the payload, then the payload: a type byte and its fields, little-endian.
//!
//! | Type | Record     | Fields                                               |
//! | ---- | ---------- | ---------------------------------------------------- |
//! | 1    | edits      | count `u32`; per edit `x`, `y`, `z` (`i32`) and material `u16` |
//! | 2    | fill       | box min `x`, `y`, `z` and end `x`, `y`, `z` (`i32`); material `u16` |
//! | 3    | checkpoint | sequence `u64`: everything before is in region files |
//!
//! A crash can tear the last record. Reading stops at the first record that
//! is short or fails its checksum, and reports how many bytes were good, so
//! the writer truncates the tail and carries on.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use xxhash_rust::xxh3::xxh3_64;

use crate::coords::{VoxelBox, VoxelPos};
use crate::edit::Edit;
use crate::material::MaterialId;

/// The first bytes of every edit log.
pub const WAL_MAGIC: [u8; 4] = *b"WVL\0";

/// The current edit log version.
pub const WAL_VERSION: u16 = 1;

const HEADER_LEN: usize = 8;
const RECORD_HEAD: usize = 12;

/// The largest payload a reader accepts.
pub const MAX_PAYLOAD: u32 = 64 << 20;

const EDIT_LEN: usize = 14;

/// One logged step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WalRecord {
    /// A batch for `VoxelTree::apply`.
    Edits(Vec<Edit>),
    /// A box for `VoxelTree::fill_box`.
    Fill {
        region: VoxelBox,
        material: MaterialId,
    },
    /// Everything logged before is saved in region files.
    Checkpoint(u64),
}

/// Why a log could not be read at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WalError {
    /// The header is missing or wrong.
    NotALog,
}

impl fmt::Display for WalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not an edit log")
    }
}

impl std::error::Error for WalError {}

fn header() -> [u8; HEADER_LEN] {
    let mut h = [0u8; HEADER_LEN];
    h[..4].copy_from_slice(&WAL_MAGIC);
    h[4..6].copy_from_slice(&WAL_VERSION.to_le_bytes());
    h
}

fn put_pos(out: &mut Vec<u8>, p: VoxelPos) {
    for v in p.to_array() {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

/// The payload of one record.
fn payload(record: &WalRecord) -> Vec<u8> {
    let mut out = Vec::new();
    match record {
        WalRecord::Edits(edits) => {
            out.reserve(5 + EDIT_LEN * edits.len());
            out.push(1);
            out.extend_from_slice(&(edits.len() as u32).to_le_bytes());
            for e in edits {
                put_pos(&mut out, e.pos);
                out.extend_from_slice(&e.material.0.to_le_bytes());
            }
        }
        WalRecord::Fill { region, material } => {
            out.push(2);
            put_pos(&mut out, region.min());
            put_pos(&mut out, region.end());
            out.extend_from_slice(&material.0.to_le_bytes());
        }
        WalRecord::Checkpoint(sequence) => {
            out.push(3);
            out.extend_from_slice(&sequence.to_le_bytes());
        }
    }
    out
}

/// One whole record: length, checksum, payload.
pub fn encode_record(record: &WalRecord) -> Vec<u8> {
    let payload = payload(record);
    let mut out = Vec::with_capacity(RECORD_HEAD + payload.len());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&xxh3_64(&payload).to_le_bytes());
    out.extend_from_slice(&payload);
    out
}

/// Parses one payload; `None` if it is malformed.
fn parse(p: &[u8]) -> Option<WalRecord> {
    let i32_at = |at: usize| Some(i32::from_le_bytes(p.get(at..at + 4)?.try_into().ok()?));
    let pos_at = |at: usize| Some(VoxelPos::new(i32_at(at)?, i32_at(at + 4)?, i32_at(at + 8)?));
    let u16_at = |at: usize| Some(u16::from_le_bytes(p.get(at..at + 2)?.try_into().ok()?));
    match *p.first()? {
        1 => {
            let count = u32::from_le_bytes(p.get(1..5)?.try_into().ok()?) as usize;
            if p.len() != 5 + EDIT_LEN * count {
                return None;
            }
            let edits = (0..count)
                .map(|k| {
                    let at = 5 + EDIT_LEN * k;
                    Some(Edit::new(pos_at(at)?, MaterialId(u16_at(at + 12)?)))
                })
                .collect::<Option<Vec<_>>>()?;
            Some(WalRecord::Edits(edits))
        }
        2 if p.len() == 27 => Some(WalRecord::Fill {
            region: VoxelBox::new(pos_at(1)?, pos_at(13)?),
            material: MaterialId(u16_at(25)?),
        }),
        3 if p.len() == 9 => Some(WalRecord::Checkpoint(u64::from_le_bytes(
            p.get(1..9)?.try_into().ok()?,
        ))),
        _ => None,
    }
}

/// Reads a whole log: every intact record in order, and the length of the
/// intact prefix (a torn or corrupt tail is left out).
pub fn read_log(bytes: &[u8]) -> Result<(Vec<WalRecord>, usize), WalError> {
    if bytes.len() < HEADER_LEN || bytes[..HEADER_LEN] != header() {
        return Err(WalError::NotALog);
    }
    let mut records = Vec::new();
    let mut at = HEADER_LEN;
    while let Some(head) = bytes.get(at..at + RECORD_HEAD) {
        let len = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
        if len > MAX_PAYLOAD {
            break;
        }
        let sum = u64::from_le_bytes(std::array::from_fn(|i| head[4 + i]));
        let start = at + RECORD_HEAD;
        let Some(payload) = bytes.get(start..start + len as usize) else {
            break;
        };
        if xxh3_64(payload) != sum {
            break;
        }
        let Some(record) = parse(payload) else {
            break;
        };
        records.push(record);
        at = start + len as usize;
    }
    Ok((records, at))
}

/// Appends records to a log file.
pub struct WalWriter {
    file: File,
    path: PathBuf,
}

impl WalWriter {
    /// Opens the log at `path`, creating it if needed. An existing log is
    /// checked and its torn tail cut off; returns the records it held.
    pub fn open(path: &Path) -> io::Result<(Self, Vec<WalRecord>)> {
        let existing = match std::fs::read(path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        let (records, good) = match &existing {
            Some(bytes) => {
                read_log(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
            }
            None => (Vec::new(), 0),
        };
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(path)?;
        if existing.is_none() {
            file.write_all(&header())?;
        } else {
            file.set_len(good as u64)?;
        }
        use std::io::Seek;
        file.seek(io::SeekFrom::End(0))?;
        Ok((
            Self {
                file,
                path: path.to_path_buf(),
            },
            records,
        ))
    }

    /// Appends one record. Call [`sync`](Self::sync) to make it durable.
    pub fn append(&mut self, record: &WalRecord) -> io::Result<()> {
        self.file.write_all(&encode_record(record))
    }

    /// Flushes appended records to the disk.
    pub fn sync(&mut self) -> io::Result<()> {
        self.file.sync_data()
    }

    /// Empties the log once everything in it is saved in region files.
    pub fn reset(&mut self) -> io::Result<()> {
        self.file.set_len(HEADER_LEN as u64)?;
        use std::io::Seek;
        self.file.seek(io::SeekFrom::End(0))?;
        self.file.sync_data()
    }

    /// Replaces the whole log with `records`, atomically: they are written
    /// to a temporary file beside it and synced, which is then renamed over
    /// the log (and the directory synced on Unix). Later appends go to the
    /// new log. If anything fails before the rename, the old log is left as
    /// it was and appends keep going to it.
    pub fn replace(&mut self, records: &[WalRecord]) -> io::Result<()> {
        let mut tmp_name = self.path.as_os_str().to_owned();
        tmp_name.push(".tmp");
        let tmp_path = PathBuf::from(tmp_name);
        let written = (|| {
            let mut tmp = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&tmp_path)?;
            let mut bytes = header().to_vec();
            for record in records {
                bytes.extend_from_slice(&encode_record(record));
            }
            tmp.write_all(&bytes)?;
            tmp.sync_all()?;
            Ok(tmp)
        })();
        let tmp = match written {
            Ok(tmp) => tmp,
            Err(e) => {
                let _ = std::fs::remove_file(&tmp_path);
                return Err(e);
            }
        };
        // The old handle is closed before the rename (Windows refuses to
        // replace a file that is open without delete sharing); the new
        // handle, positioned at its end, follows the file through it.
        let old = std::mem::replace(&mut self.file, tmp);
        drop(old);
        if let Err(e) = std::fs::rename(&tmp_path, &self.path) {
            let _ = std::fs::remove_file(&tmp_path);
            let reopened = OpenOptions::new().write(true).open(&self.path);
            match reopened {
                Ok(mut file) => {
                    use std::io::Seek;
                    file.seek(io::SeekFrom::End(0))?;
                    self.file = file;
                }
                Err(reopen) => {
                    return Err(io::Error::new(
                        e.kind(),
                        format!("{e}; reopening the old log: {reopen}"),
                    ));
                }
            }
            return Err(e);
        }
        #[cfg(unix)]
        if let Some(dir) = self.path.parent() {
            let dir = if dir.as_os_str().is_empty() {
                Path::new(".")
            } else {
                dir
            };
            File::open(dir)?.sync_all()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<WalRecord> {
        vec![
            WalRecord::Edits(vec![
                Edit::new(VoxelPos::new(-5, 7, 1 << 19), MaterialId(3)),
                Edit::new(VoxelPos::new(0, 0, 0), MaterialId::AIR),
            ]),
            WalRecord::Fill {
                region: VoxelBox::new(VoxelPos::new(-10, -10, -10), VoxelPos::new(10, 0, 4)),
                material: MaterialId::AIR,
            },
            WalRecord::Edits(Vec::new()),
            WalRecord::Checkpoint(42),
        ]
    }

    fn log_of(records: &[WalRecord]) -> Vec<u8> {
        let mut bytes = header().to_vec();
        for r in records {
            bytes.extend_from_slice(&encode_record(r));
        }
        bytes
    }

    #[test]
    fn records_round_trip_in_order() {
        let bytes = log_of(&sample());
        let (records, good) = read_log(&bytes).unwrap();
        assert_eq!(records, sample());
        assert_eq!(good, bytes.len());
    }

    #[test]
    fn a_torn_tail_keeps_every_whole_record_before_it() {
        let bytes = log_of(&sample());
        let whole = log_of(&sample()[..2]).len();
        for cut in whole..bytes.len() {
            let (records, good) = read_log(&bytes[..cut]).unwrap();
            assert!(records.len() >= 2, "cut at {cut}");
            assert!(good <= cut);
        }
        let mut corrupt = bytes.clone();
        let last = corrupt.len() - 1;
        corrupt[last] ^= 1;
        let (records, _) = read_log(&corrupt).unwrap();
        assert_eq!(records, sample()[..3]);
        assert_eq!(read_log(b"WVL"), Err(WalError::NotALog));
    }

    #[test]
    fn the_writer_appends_recovers_and_resets() {
        let dir = std::env::temp_dir().join(format!("world_wal_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("edits.wvl");
        let _ = std::fs::remove_file(&path);
        {
            let (mut w, old) = WalWriter::open(&path).unwrap();
            assert!(old.is_empty());
            for r in &sample() {
                w.append(r).unwrap();
            }
            w.sync().unwrap();
        }
        // Simulate a torn write, then reopen.
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(&[9, 0, 0, 0, 1, 2]);
        std::fs::write(&path, &bytes).unwrap();
        let (mut w, old) = WalWriter::open(&path).unwrap();
        assert_eq!(old, sample());
        w.append(&WalRecord::Checkpoint(7)).unwrap();
        w.sync().unwrap();
        drop(w);
        let (records, _) = read_log(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(records.last(), Some(&WalRecord::Checkpoint(7)));
        assert_eq!(records.len(), sample().len() + 1);

        let (mut w, _) = WalWriter::open(&path).unwrap();
        w.reset().unwrap();
        drop(w);
        let (records, _) = read_log(&std::fs::read(&path).unwrap()).unwrap();
        assert!(records.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn replace_swaps_the_whole_log_and_appends_follow_it() {
        let dir = std::env::temp_dir().join(format!("world_wal_replace_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("edits.wvl");
        let (mut w, _) = WalWriter::open(&path).unwrap();
        for r in &sample() {
            w.append(r).unwrap();
        }
        w.sync().unwrap();
        let kept = vec![WalRecord::Checkpoint(3), sample()[1].clone()];
        w.replace(&kept).unwrap();
        w.append(&sample()[0]).unwrap();
        w.sync().unwrap();
        drop(w);
        let (records, good) = read_log(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(records, [kept, vec![sample()[0].clone()]].concat());
        assert_eq!(good, std::fs::metadata(&path).unwrap().len() as usize);
        assert!(!dir.join("edits.wvl.tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_replace_leaves_the_old_log_and_keeps_appending_to_it() {
        let dir = std::env::temp_dir().join(format!("world_wal_fail_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("edits.wvl");
        let (mut w, _) = WalWriter::open(&path).unwrap();
        for r in &sample() {
            w.append(r).unwrap();
        }
        w.sync().unwrap();
        // A directory where the temporary file goes makes the write fail.
        std::fs::create_dir_all(dir.join("edits.wvl.tmp")).unwrap();
        assert!(w.replace(&[WalRecord::Checkpoint(1)]).is_err());
        w.append(&WalRecord::Checkpoint(9)).unwrap();
        w.sync().unwrap();
        drop(w);
        let (records, _) = read_log(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(records, [sample(), vec![WalRecord::Checkpoint(9)]].concat());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
