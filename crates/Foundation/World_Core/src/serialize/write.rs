//! Writing the body: the tree depth-first, one tagged child at a time.

use xxhash_rust::xxh3::Xxh3;

use super::{TAG_AIR, TAG_BRANCH, TAG_LEAF, TAG_UNIFORM, index_bits};
use crate::tree::{NodeKind, NodeView};

/// Where body bytes go: a buffer when encoding, a hasher for the content hash.
pub(super) trait Out {
    fn put(&mut self, bytes: &[u8]);
}

impl Out for Vec<u8> {
    fn put(&mut self, bytes: &[u8]) {
        self.extend_from_slice(bytes);
    }
}

impl Out for Xxh3 {
    fn put(&mut self, bytes: &[u8]) {
        self.update(bytes);
    }
}

/// Writes one child and everything below it.
pub(super) fn child(out: &mut impl Out, node: impl NodeView) {
    match node.kind() {
        NodeKind::Air => out.put(&[TAG_AIR]),
        NodeKind::Uniform(m) => {
            out.put(&[TAG_UNIFORM]);
            out.put(&m.0.to_le_bytes());
        }
        NodeKind::Branch => {
            out.put(&[TAG_BRANCH]);
            out.put(&node.mask().to_le_bytes());
            for (_, c) in node.children() {
                child(out, c);
            }
        }
        NodeKind::Leaf => leaf(out, node),
    }
}

fn leaf(out: &mut impl Out, node: impl NodeView) {
    let mask = node.mask();
    let palette = node.palette();
    let palette = palette.as_slice();

    out.put(&[TAG_LEAF]);
    out.put(&mask.to_le_bytes());
    out.put(&[palette.len() as u8]);
    for m in palette {
        out.put(&m.0.to_le_bytes());
    }

    // The index width divides 8, so indices never straddle a byte.
    let bits = index_bits(palette.len());
    if bits == 0 {
        return;
    }
    let mut packed = [0u8; 64];
    let mut bit = 0;
    for slot in (0..64).filter(|s| mask >> s & 1 == 1) {
        let index = palette
            .binary_search(&node.voxel(slot))
            .expect("every solid voxel of a leaf is in its palette");
        packed[bit / 8] |= (index as u8) << (bit % 8);
        bit += bits;
    }
    out.put(&packed[..bit.div_ceil(8)]);
}
