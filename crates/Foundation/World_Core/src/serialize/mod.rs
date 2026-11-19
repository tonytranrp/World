//! Binary encoding of trees (decision 0004).
//!
//! Layout: a header (magic, format version, tree depth, body length), the body
//! (the tree depth-first from the root, little-endian), then the xxh3-64 of the
//! body. Trees are canonical, so equal content gives identical bytes and the
//! body hash is a content hash.

// SKELETON: the bodies marked `todo!()` are filled in by the serialization
// implementation; the signatures are the contract.

use std::fmt;

use crate::tree::{TreeRead, VoxelTree};

/// The first bytes of every encoded tree.
pub const MAGIC: [u8; 4] = *b"WVT\0";

/// The current format version. Every layout change bumps it.
pub const FORMAT_VERSION: u16 = 1;

/// Encodes a tree. The same content always gives the same bytes.
pub fn encode(tree: &impl TreeRead) -> Vec<u8> {
    let _ = tree;
    todo!()
}

/// Decodes bytes made by [`encode`] into a canonical tree. Corrupt, truncated
/// or hostile input returns an error; it never panics.
pub fn decode(bytes: &[u8]) -> Result<VoxelTree, DecodeError> {
    let _ = bytes;
    todo!()
}

/// A hash of the tree's content, stable across machines and versions of this
/// crate that share the format version: the xxh3-64 of the encoded body.
pub fn content_hash(tree: &impl TreeRead) -> u64 {
    let _ = tree;
    todo!()
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
