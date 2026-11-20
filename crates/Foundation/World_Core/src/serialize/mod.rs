//! Binary encoding of trees (decision 0004).
//!
//! # Format, version 1
//!
//! Every multi-byte integer is little-endian. An encoded tree is a header, a
//! body and a checksum, with nothing before or after:
//!
//! | Offset          | Size | Field                                             |
//! | --------------- | ---- | ------------------------------------------------- |
//! | 0               | 4    | [`MAGIC`], `b"WVT\0"`                             |
//! | 4               | 2    | format version, `u16`, [`FORMAT_VERSION`]         |
//! | 6               | 1    | tree depth, `u8`, [`TREE_DEPTH`]                  |
//! | 7               | 8    | body length `n` in bytes, `u64`                   |
//! | 15              | `n`  | body                                              |
//! | 15 + `n`        | 8    | xxh3-64 of the body, `u64`                        |
//!
//! The body length is a `u64` so that no tree, however large, is unencodable;
//! the decoder never trusts it beyond the bytes actually present.
//!
//! The body is the root, written as one *child*. A child is a tag byte and the
//! tag's payload:
//!
//! | Tag | Child   | Payload                                                   |
//! | --- | ------- | --------------------------------------------------------- |
//! | 0   | air     | none. Only the root may be air; air children of a branch are clear mask bits |
//! | 1   | uniform | material id, `u16`, not 0                                 |
//! | 2   | branch  | mask, `u64`, not 0; then each child whose mask bit is set, in ascending slot order |
//! | 3   | leaf    | mask, `u64`, not 0; palette length `p`, `u8`; `p` material ids, `u16`; packed indices |
//!
//! The root child stands where a level `TREE_DEPTH - 1` node would; the
//! children of a level-`L` branch stand where level `L - 1` nodes would. A
//! branch may only stand at level 1 or above and a leaf only at level 0. A
//! uniform child covers the whole region of the node it replaces.
//!
//! A leaf's mask bit `i` is set when voxel slot `i` is solid. Its palette is
//! strictly ascending and holds no air. The indices give, for each set mask
//! bit in ascending slot order, the voxel's position in the palette, packed
//! LSB-first into `ceil(popcount(mask) * w / 8)` bytes, where `w` is the
//! smallest of 0, 1, 2, 4 or 8 bits that can index `p` entries (so one
//! material takes no index bytes). Bits past the last index are zero.
//!
//! The body must be canonical, exactly as the tree is: no branch or leaf whose
//! voxels are all one material (a full mask of identical uniform children, or
//! a full leaf with one palette entry), every palette entry used by some
//! voxel, and no bytes after the root. Equal content therefore has exactly one
//! encoding, and the xxh3-64 of the body is the [`content_hash`].
//!
//! # Decoding
//!
//! [`decode`] checks the header and checksum, then validates the whole body
//! before it builds anything, so a rejected input never costs more than one
//! pass over its own bytes. It allocates nothing but the tree it returns.

mod read;
mod write;

use std::fmt;

use xxhash_rust::xxh3::{Xxh3, xxh3_64};

use crate::consts::TREE_DEPTH;
use crate::tree::{TreeRead, VoxelTree};

/// The first bytes of every encoded tree.
pub const MAGIC: [u8; 4] = *b"WVT\0";

/// The current format version. Every layout change bumps it.
pub const FORMAT_VERSION: u16 = 1;

/// Bytes before the body: magic, version, depth and body length.
const HEADER_LEN: usize = 15;

/// Bytes after the body: its xxh3-64.
const CHECKSUM_LEN: usize = 8;

const _: () = assert!(TREE_DEPTH <= u8::MAX as u32);

/// Child tags.
const TAG_AIR: u8 = 0;
const TAG_UNIFORM: u8 = 1;
const TAG_BRANCH: u8 = 2;
const TAG_LEAF: u8 = 3;

/// Encodes a tree. The same content always gives the same bytes.
pub fn encode(tree: &impl TreeRead) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(HEADER_LEN + 1 + CHECKSUM_LEN);
    bytes.extend_from_slice(&MAGIC);
    bytes.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    bytes.push(TREE_DEPTH as u8);
    bytes.extend_from_slice(&[0; 8]); // The body length, patched below.
    write::child(&mut bytes, tree.root());
    let body_len = (bytes.len() - HEADER_LEN) as u64;
    bytes[HEADER_LEN - 8..HEADER_LEN].copy_from_slice(&body_len.to_le_bytes());
    let checksum = xxh3_64(&bytes[HEADER_LEN..]);
    bytes.extend_from_slice(&checksum.to_le_bytes());
    bytes
}

/// Decodes bytes made by [`encode`] into a canonical tree. Corrupt, truncated
/// or hostile input returns an error; it never panics.
pub fn decode(bytes: &[u8]) -> Result<VoxelTree, DecodeError> {
    let body = unseal(bytes)?;
    read::body(body, &mut read::Validate)?;
    // The body is known to be valid, so the build pass cannot fail; it
    // writes each leaf and node once, bottom-up.
    VoxelTree::build_with(|builder| read::body(body, &mut read::TreeBuild { builder }))
}

/// A hash of the tree's content, stable across machines and versions of this
/// crate that share the format version: the xxh3-64 of the encoded body.
pub fn content_hash(tree: &impl TreeRead) -> u64 {
    let mut hasher = Xxh3::new();
    write::child(&mut hasher, tree.root());
    hasher.digest()
}

/// Checks everything around the body (magic, version, depth, length and
/// checksum) and returns the body.
fn unseal(bytes: &[u8]) -> Result<&[u8], DecodeError> {
    let prefix = bytes.len().min(MAGIC.len());
    if bytes[..prefix] != MAGIC[..prefix] {
        return Err(DecodeError::NotATree);
    }
    let Some(&[_, _, _, _, v0, v1, depth, ref len @ ..]) = bytes.first_chunk::<HEADER_LEN>() else {
        return Err(DecodeError::Truncated);
    };
    let version = u16::from_le_bytes([v0, v1]);
    if version != FORMAT_VERSION {
        return Err(DecodeError::UnsupportedVersion { found: version });
    }
    if u32::from(depth) != TREE_DEPTH {
        return Err(DecodeError::WrongDepth { found: depth });
    }
    let body_len = u64::from_le_bytes(*len);
    let Some(room) = (bytes.len() - HEADER_LEN).checked_sub(CHECKSUM_LEN) else {
        return Err(DecodeError::Truncated);
    };
    let room = room as u64;
    if body_len > room {
        return Err(DecodeError::Truncated);
    }
    if body_len < room {
        return Err(DecodeError::TrailingBytes);
    }
    let (body, checksum) = bytes[HEADER_LEN..].split_at(room as usize);
    if xxh3_64(body).to_le_bytes() != checksum {
        return Err(DecodeError::ChecksumMismatch);
    }
    Ok(body)
}

/// Why bytes could not be decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DecodeError {
    /// The input does not start with [`MAGIC`].
    NotATree,
    /// The input uses a format version this build cannot read.
    UnsupportedVersion { found: u16 },
    /// The input was written for a tree of another depth.
    WrongDepth { found: u8 },
    /// The input ends before the data it declares.
    Truncated,
    /// Bytes follow the end of the encoded tree.
    TrailingBytes,
    /// The body does not match its checksum.
    ChecksumMismatch,
    /// The body breaks the format: the message says how and the offset where.
    Malformed { offset: usize, reason: &'static str },
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotATree => write!(f, "not an encoded voxel tree"),
            Self::UnsupportedVersion { found } => write!(
                f,
                "format version {found} is not supported (this build reads {FORMAT_VERSION})"
            ),
            Self::WrongDepth { found } => write!(f, "tree depth {found} does not match this build"),
            Self::Truncated => write!(f, "the data ends early"),
            Self::TrailingBytes => write!(f, "unexpected bytes after the tree"),
            Self::ChecksumMismatch => write!(f, "the checksum does not match: the data is corrupt"),
            Self::Malformed { offset, reason } => write!(f, "malformed at byte {offset}: {reason}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// The smallest of 0, 1, 2, 4 or 8 bits that can index `len` palette entries.
const fn index_bits(len: usize) -> usize {
    match len {
        0 | 1 => 0,
        2 => 1,
        3..=4 => 2,
        5..=16 => 4,
        _ => 8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::material::MaterialId;
    use crate::tree::NodeRef;

    /// A tree that is a single child value, which needs no node store.
    struct Whole(NodeRef<'static>);

    impl TreeRead for Whole {
        fn root(&self) -> NodeRef<'_> {
            self.0
        }
    }

    /// Wraps a body in a valid header and checksum.
    fn seal(body: &[u8]) -> Vec<u8> {
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        bytes.push(TREE_DEPTH as u8);
        bytes.extend_from_slice(&(body.len() as u64).to_le_bytes());
        bytes.extend_from_slice(body);
        bytes.extend_from_slice(&xxh3_64(body).to_le_bytes());
        bytes
    }

    #[test]
    fn an_air_root_is_one_tag_byte() {
        let bytes = encode(&Whole(NodeRef::Air));
        assert_eq!(bytes, seal(&[TAG_AIR]));
        assert_eq!(content_hash(&Whole(NodeRef::Air)), xxh3_64(&[TAG_AIR]));
    }

    #[test]
    fn a_uniform_root_is_its_tag_and_id() {
        let root = Whole(NodeRef::Uniform(MaterialId(0x0102)));
        assert_eq!(encode(&root), seal(&[TAG_UNIFORM, 0x02, 0x01]));
        assert_eq!(content_hash(&root), xxh3_64(&[TAG_UNIFORM, 0x02, 0x01]));
    }

    #[test]
    fn the_header_is_exactly_as_documented() {
        let bytes = encode(&Whole(NodeRef::Air));
        assert_eq!(bytes.len(), HEADER_LEN + 1 + CHECKSUM_LEN);
        assert_eq!(&bytes[..4], b"WVT\0");
        assert_eq!(&bytes[4..6], &[1, 0]);
        assert_eq!(bytes[6], 10);
        assert_eq!(&bytes[7..15], &[1, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn an_air_root_decodes() {
        assert!(decode(&seal(&[TAG_AIR])).is_ok());
    }

    #[test]
    fn header_errors_are_specific() {
        let good = seal(&[TAG_AIR]);
        assert_eq!(decode(b"").err(), Some(DecodeError::Truncated));
        assert_eq!(decode(b"WV").err(), Some(DecodeError::Truncated));
        assert_eq!(decode(b"WX").err(), Some(DecodeError::NotATree));
        assert_eq!(decode(b"PNG\0 and more").err(), Some(DecodeError::NotATree));

        let mut v = good.clone();
        v[4] = 2;
        assert_eq!(
            decode(&v).err(),
            Some(DecodeError::UnsupportedVersion { found: 2 })
        );
        let mut d = good.clone();
        d[6] = 9;
        assert_eq!(decode(&d).err(), Some(DecodeError::WrongDepth { found: 9 }));

        let mut long = good.clone();
        long.push(0);
        assert_eq!(decode(&long).err(), Some(DecodeError::TrailingBytes));
        let mut huge = good.clone();
        huge[7..15].copy_from_slice(&u64::MAX.to_le_bytes());
        assert_eq!(decode(&huge).err(), Some(DecodeError::Truncated));

        let mut sum = good;
        *sum.last_mut().unwrap() ^= 1;
        assert_eq!(decode(&sum).err(), Some(DecodeError::ChecksumMismatch));
    }

    #[test]
    fn every_truncation_of_a_valid_encoding_fails() {
        let good = seal(&[TAG_UNIFORM, 5, 0]);
        for len in 0..good.len() {
            assert!(decode(&good[..len]).is_err(), "length {len}");
        }
    }

    #[test]
    fn index_width_is_the_smallest_power_of_two_that_fits() {
        let widths = [1, 2, 3, 4, 5, 16, 17, 64].map(index_bits);
        assert_eq!(widths, [0, 1, 2, 2, 4, 4, 8, 8]);
    }
}
