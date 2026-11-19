//! Shared by the fuzz targets: turning arbitrary bytes into an input whose
//! header and checksum are valid, so mutations reach the body parser instead
//! of stopping at the checksum.

#![allow(dead_code)] // Each target uses a different part.

use world_core::consts::TREE_DEPTH;
use world_core::serialize::{FORMAT_VERSION, MAGIC};
use xxhash_rust::xxh3::xxh3_64;

/// Header bytes before the body, and checksum bytes after it (format v1).
pub const HEADER_LEN: usize = 15;
pub const CHECKSUM_LEN: usize = 8;

/// The body inside `data`: between the header and the checksum if `data`
/// looks like an encoding (as the seeds are), otherwise all of `data`.
pub fn body_of(data: &[u8]) -> &[u8] {
    if data.len() >= HEADER_LEN + CHECKSUM_LEN && data.starts_with(&MAGIC) {
        &data[HEADER_LEN..data.len() - CHECKSUM_LEN]
    } else {
        data
    }
}

/// `body` with a correct header (magic, version, depth, length) and checksum.
pub fn seal(body: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(HEADER_LEN + body.len() + CHECKSUM_LEN);
    bytes.extend_from_slice(&MAGIC);
    bytes.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    bytes.push(TREE_DEPTH as u8);
    bytes.extend_from_slice(&(body.len() as u64).to_le_bytes());
    bytes.extend_from_slice(body);
    bytes.extend_from_slice(&xxh3_64(body).to_le_bytes());
    bytes
}
