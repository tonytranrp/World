//! Region files: the saved chunks of one 256 m region (decision 0006).
//!
//! A region is the cube under a level-6 node: 16 x 16 x 16 chunks. Only
//! chunks that differ from the generated terrain are saved, each as one or
//! more records: its edit overlay or its full content, plus coarse summaries
//! for distant detail. Each record holds an encoded tree (see the parent
//! module), compressed.
//!
//! # Format, version 1
//!
//! Little-endian throughout:
//!
//! | Offset | Size     | Field                                          |
//! | ------ | -------- | ---------------------------------------------- |
//! | 0      | 4        | [`REGION_MAGIC`], `b"WVR\0"`                   |
//! | 4      | 2        | version, [`REGION_VERSION`]                    |
//! | 6      | 2        | zero                                           |
//! | 8      | 12       | region coordinate, three `u32`                 |
//! | 20     | 4        | record count `n`                               |
//! | 24     | 40 n     | the table, sorted by (slot, kind)              |
//! | 24+40n | 8        | xxh3-64 of every byte before it                |
//! |        |          | the records' bytes, in table order             |
//!
//! A table entry is: chunk slot `u16` (`x + 16y + 256z` inside the region),
//! kind `u8`, codec `u8`, generator version `u32`, stored length `u32`,
//! decoded length `u32`, offset `u64` from the start of the file, xxh3-64 of
//! the stored bytes, and 8 zero bytes. Readers check everything and never
//! trust a length beyond the bytes present or [`MAX_RECORD_LEN`].
//!
//! Files are written whole to a temporary name and renamed over the old one,
//! so a crash leaves either the old file or the new one, never a mix.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use xxhash_rust::xxh3::xxh3_64;

use super::codec::{self, Codec};

/// The first bytes of every region file.
pub const REGION_MAGIC: [u8; 4] = *b"WVR\0";

/// The current region format version.
pub const REGION_VERSION: u16 = 1;

/// Chunks per region along each axis.
pub const REGION_CHUNKS: u32 = 16;

/// The largest decoded record a reader accepts: a full chunk of the densest
/// content stays well below it.
pub const MAX_RECORD_LEN: u32 = 1 << 30;

const HEADER_LEN: usize = 24;
const ENTRY_LEN: usize = 40;
const SUM_LEN: usize = 8;

/// What a record holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RecordKind {
    /// The chunk's edit overlay against its generated base.
    Overlay,
    /// The chunk's whole content.
    Full,
    /// A coarse summary with cells of `4^level` voxels.
    Coarse(u8),
}

impl RecordKind {
    const fn byte(self) -> u8 {
        match self {
            Self::Overlay => 0,
            Self::Full => 1,
            Self::Coarse(level) => 16 + level,
        }
    }

    const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Overlay),
            1 => Some(Self::Full),
            16..=31 => Some(Self::Coarse(byte - 16)),
            _ => None,
        }
    }
}

/// One saved record before compression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkRecord {
    /// The chunk inside the region, each axis 0 to 15.
    pub chunk: [u32; 3],
    pub kind: RecordKind,
    /// The generator version the base terrain came from.
    pub generator: u32,
    /// The encoded tree.
    pub bytes: Vec<u8>,
}

/// A table entry of a region file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordEntry {
    pub chunk: [u32; 3],
    pub kind: RecordKind,
    pub codec: Codec,
    pub generator: u32,
    pub stored_len: u32,
    pub decoded_len: u32,
    pub offset: u64,
    pub checksum: u64,
}

/// Why a region file could not be read.
#[derive(Debug)]
pub enum RegionError {
    Io(io::Error),
    /// The file breaks the format; the message says how.
    Corrupt(&'static str),
}

impl fmt::Display for RegionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "region file I/O: {e}"),
            Self::Corrupt(why) => write!(f, "corrupt region file: {why}"),
        }
    }
}

impl std::error::Error for RegionError {}

impl From<io::Error> for RegionError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

fn slot(chunk: [u32; 3]) -> u16 {
    (chunk[0] + REGION_CHUNKS * (chunk[1] + REGION_CHUNKS * chunk[2])) as u16
}

fn chunk_of(slot: u16) -> [u32; 3] {
    let s = u32::from(slot);
    [s % 16, s / 16 % 16, s / 256]
}

/// Encodes a whole region file. Records are sorted by chunk and kind; a
/// repeated (chunk, kind) keeps the last one given.
pub fn encode_region(region: [u32; 3], records: &[ChunkRecord], codec: Codec) -> Vec<u8> {
    let mut sorted: Vec<&ChunkRecord> = records.iter().collect();
    sorted.sort_by_key(|r| (slot(r.chunk), r.kind));
    sorted.dedup_by(|later, kept| {
        let same = (slot(later.chunk), later.kind) == (slot(kept.chunk), kept.kind);
        if same {
            *kept = *later;
        }
        same
    });

    let blobs: Vec<Vec<u8>> = sorted
        .iter()
        .map(|r| codec::compress(codec, &r.bytes))
        .collect();
    let table_end = HEADER_LEN + ENTRY_LEN * sorted.len() + SUM_LEN;
    let mut out = Vec::with_capacity(table_end + blobs.iter().map(Vec::len).sum::<usize>());
    out.extend_from_slice(&REGION_MAGIC);
    out.extend_from_slice(&REGION_VERSION.to_le_bytes());
    out.extend_from_slice(&[0; 2]);
    for v in region {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&(sorted.len() as u32).to_le_bytes());
    let mut offset = table_end as u64;
    for (record, blob) in sorted.iter().zip(&blobs) {
        out.extend_from_slice(&slot(record.chunk).to_le_bytes());
        out.push(record.kind.byte());
        out.push(codec as u8);
        out.extend_from_slice(&record.generator.to_le_bytes());
        out.extend_from_slice(&(blob.len() as u32).to_le_bytes());
        out.extend_from_slice(&(record.bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&xxh3_64(blob).to_le_bytes());
        out.extend_from_slice(&[0; 8]);
        offset += blob.len() as u64;
    }
    let sum = xxh3_64(&out);
    out.extend_from_slice(&sum.to_le_bytes());
    for blob in &blobs {
        out.extend_from_slice(blob);
    }
    out
}

/// The table of a region file: its coordinate and record entries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegionIndex {
    pub region: [u32; 3],
    pub entries: Vec<RecordEntry>,
}

impl RegionIndex {
    /// The entry of a chunk's record of `kind`, if saved.
    pub fn find(&self, chunk: [u32; 3], kind: RecordKind) -> Option<&RecordEntry> {
        self.entries
            .iter()
            .find(|e| e.chunk == chunk && e.kind == kind)
    }
}

/// Reads and checks the header and table from `reader`, positioned at the
/// start of the file. `file_len` bounds every offset.
pub fn read_index(reader: &mut impl Read, file_len: u64) -> Result<RegionIndex, RegionError> {
    let mut header = [0u8; HEADER_LEN];
    reader
        .read_exact(&mut header)
        .map_err(|_| RegionError::Corrupt("too short"))?;
    if header[..4] != REGION_MAGIC {
        return Err(RegionError::Corrupt("not a region file"));
    }
    if u16::from_le_bytes([header[4], header[5]]) != REGION_VERSION {
        return Err(RegionError::Corrupt("unsupported version"));
    }
    let u32_at = |b: &[u8], at: usize| u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
    let region = [u32_at(&header, 8), u32_at(&header, 12), u32_at(&header, 16)];
    let count = u32_at(&header, 20) as usize;
    // Every entry needs its table bytes, so a count beyond the file is a lie.
    if (HEADER_LEN + ENTRY_LEN * count + SUM_LEN) as u64 > file_len {
        return Err(RegionError::Corrupt("the table runs past the end"));
    }
    let mut table = vec![0u8; ENTRY_LEN * count + SUM_LEN];
    reader
        .read_exact(&mut table)
        .map_err(|_| RegionError::Corrupt("too short"))?;
    let (entries_bytes, sum) = table.split_at(ENTRY_LEN * count);
    let mut summed = header.to_vec();
    summed.extend_from_slice(entries_bytes);
    if xxh3_64(&summed).to_le_bytes() != sum {
        return Err(RegionError::Corrupt("the table checksum does not match"));
    }
    let mut entries = Vec::with_capacity(count);
    for e in entries_bytes.chunks_exact(ENTRY_LEN) {
        let u64_at = |at: usize| u64::from_le_bytes(std::array::from_fn(|i| e[at + i]));
        let entry = RecordEntry {
            chunk: chunk_of(u16::from_le_bytes([e[0], e[1]])),
            kind: RecordKind::from_byte(e[2]).ok_or(RegionError::Corrupt("unknown record kind"))?,
            codec: Codec::from_byte(e[3]).ok_or(RegionError::Corrupt("unknown codec"))?,
            generator: u32_at(e, 4),
            stored_len: u32_at(e, 8),
            decoded_len: u32_at(e, 12),
            offset: u64_at(16),
            checksum: u64_at(24),
        };
        if entry.decoded_len > MAX_RECORD_LEN
            || entry.offset.saturating_add(u64::from(entry.stored_len)) > file_len
        {
            return Err(RegionError::Corrupt("a record runs past the end"));
        }
        entries.push(entry);
    }
    Ok(RegionIndex { region, entries })
}

/// Reads, checks and decompresses one record.
pub fn read_record(
    reader: &mut (impl Read + Seek),
    entry: &RecordEntry,
) -> Result<Vec<u8>, RegionError> {
    reader.seek(SeekFrom::Start(entry.offset))?;
    let mut blob = vec![0u8; entry.stored_len as usize];
    reader.read_exact(&mut blob)?;
    if xxh3_64(&blob) != entry.checksum {
        return Err(RegionError::Corrupt("a record checksum does not match"));
    }
    codec::decompress(entry.codec, &blob, entry.decoded_len as usize)
        .map_err(|e| RegionError::Corrupt(e.0))
}

/// An open region file for reading records one at a time.
pub struct RegionFile {
    file: File,
    index: RegionIndex,
}

impl RegionFile {
    /// Opens and checks a region file's table.
    pub fn open(path: &Path) -> Result<Self, RegionError> {
        let mut file = File::open(path)?;
        let len = file.metadata()?.len();
        let index = read_index(&mut file, len)?;
        Ok(Self { file, index })
    }

    pub fn index(&self) -> &RegionIndex {
        &self.index
    }

    /// The decoded bytes of a chunk's record of `kind`, if saved.
    pub fn read(
        &mut self,
        chunk: [u32; 3],
        kind: RecordKind,
    ) -> Result<Option<Vec<u8>>, RegionError> {
        match self.index.find(chunk, kind).copied() {
            Some(entry) => read_record(&mut self.file, &entry).map(Some),
            None => Ok(None),
        }
    }
}

/// Writes `bytes` to `path` through a temporary file and a rename, so the
/// file is always either the old or the new content.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = Path::new(&tmp);
    {
        let mut file = File::create(tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn records() -> Vec<ChunkRecord> {
        vec![
            ChunkRecord {
                chunk: [3, 0, 15],
                kind: RecordKind::Overlay,
                generator: 1,
                bytes: vec![7; 5000],
            },
            ChunkRecord {
                chunk: [0, 0, 0],
                kind: RecordKind::Coarse(3),
                generator: 1,
                bytes: (0..300u16).flat_map(u16::to_le_bytes).collect(),
            },
            ChunkRecord {
                chunk: [0, 0, 0],
                kind: RecordKind::Full,
                generator: 2,
                bytes: vec![],
            },
        ]
    }

    fn index_of(bytes: &[u8]) -> Result<RegionIndex, RegionError> {
        read_index(&mut Cursor::new(bytes), bytes.len() as u64)
    }

    #[test]
    fn records_round_trip_with_every_codec() {
        for codec in [Codec::Raw, Codec::Lz4, Codec::Zstd] {
            let bytes = encode_region([1, 2, 3], &records(), codec);
            let index = index_of(&bytes).unwrap();
            assert_eq!(index.region, [1, 2, 3]);
            assert_eq!(index.entries.len(), 3);
            // Sorted by slot, then kind.
            assert_eq!(index.entries[0].kind, RecordKind::Full);
            assert_eq!(index.entries[1].kind, RecordKind::Coarse(3));
            for record in records() {
                let entry = index.find(record.chunk, record.kind).unwrap();
                assert_eq!(entry.generator, record.generator);
                let back = read_record(&mut Cursor::new(&bytes), entry).unwrap();
                assert_eq!(back, record.bytes, "{codec:?}");
            }
        }
    }

    #[test]
    fn the_last_record_of_a_chunk_and_kind_wins() {
        let mut list = records();
        list.push(ChunkRecord {
            bytes: vec![9; 10],
            ..list[0].clone()
        });
        let bytes = encode_region([0; 3], &list, Codec::Lz4);
        let index = index_of(&bytes).unwrap();
        assert_eq!(index.entries.len(), 3);
        let entry = index.find([3, 0, 15], RecordKind::Overlay).unwrap();
        assert_eq!(
            read_record(&mut Cursor::new(&bytes), entry).unwrap(),
            vec![9; 10]
        );
    }

    #[test]
    fn every_flipped_byte_is_detected() {
        let bytes = encode_region([5, 5, 5], &records(), Codec::Lz4);
        for i in 0..bytes.len() {
            let mut bad = bytes.clone();
            bad[i] ^= 0x41;
            let caught = match index_of(&bad) {
                Err(_) => true,
                Ok(index) => index
                    .entries
                    .iter()
                    .any(|e| read_record(&mut Cursor::new(&bad), e).is_err()),
            };
            assert!(caught, "byte {i} flipped unnoticed");
        }
        for len in 0..bytes.len() {
            let cut = &bytes[..len];
            let caught = match read_index(&mut Cursor::new(cut), cut.len() as u64) {
                Err(_) => true,
                Ok(index) => index
                    .entries
                    .iter()
                    .any(|e| read_record(&mut Cursor::new(cut), e).is_err()),
            };
            assert!(caught, "truncation to {len} unnoticed");
        }
    }

    #[test]
    fn files_are_replaced_atomically_and_read_back() {
        let dir = std::env::temp_dir().join(format!("world_region_test_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("r.0.0.0.wvr");
        write_atomic(&path, &encode_region([0; 3], &records()[..1], Codec::Zstd)).unwrap();
        write_atomic(&path, &encode_region([0; 3], &records(), Codec::Zstd)).unwrap();
        let mut file = RegionFile::open(&path).unwrap();
        assert_eq!(file.index().entries.len(), 3);
        assert_eq!(
            file.read([0, 0, 0], RecordKind::Coarse(3))
                .unwrap()
                .unwrap(),
            records()[1].bytes
        );
        assert_eq!(file.read([1, 1, 1], RecordKind::Full).unwrap(), None);
        fs::remove_dir_all(&dir).unwrap();
    }
}
