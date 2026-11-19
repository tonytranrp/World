//! Decode, then encode: any input `decode` accepts must encode back to exactly
//! the same bytes, because a tree is canonical and has one encoding per
//! content (decision 0004), and its content hash must equal the checksum of
//! the body.
//!
//! The input's body is resealed with a valid header and checksum first (see
//! `common`), so mutations reach the body parser.

#![no_main]

mod common;

use common::{CHECKSUM_LEN, body_of, seal};
use libfuzzer_sys::fuzz_target;
use world_core::serialize::{content_hash, decode, encode};

fuzz_target!(|data: &[u8]| {
    let sealed = seal(body_of(data));
    let Ok(tree) = decode(&sealed) else {
        return;
    };
    let encoded = encode(&tree);
    assert!(
        encoded == sealed,
        "decode accepted {} bytes that encode back to {} different bytes",
        sealed.len(),
        encoded.len()
    );
    let checksum = &sealed[sealed.len() - CHECKSUM_LEN..];
    assert_eq!(
        content_hash(&tree).to_le_bytes(),
        checksum,
        "content_hash differs from the checksum of the body"
    );
});
