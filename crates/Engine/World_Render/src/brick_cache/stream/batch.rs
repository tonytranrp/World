//! One frame's upload: node writes, brick blocks and brick table writes,
//! serialized into the buffer the apply pass reads.
//!
//! # Upload buffer layout (u32 words)
//!
//! | words | content |
//! |-------|---------|
//! | 0..4 | entry counts: unlink, scatter, link, table |
//! | 4..9 | word offsets: unlink, scatter, link, table, data |
//! | 9 | total words |
//! | 10..16 | zero |
//! | unlink, link | [`NODE_WRITE_WORDS`] per entry: node index, then the 4 record words |
//! | scatter | [`SCATTER_WORDS`] per entry: source word (relative to data), packed brick ref, word count, 0 |
//! | table | [`TABLE_WRITE_WORDS`] per entry: brick id, block ref, last-used frame, 0 |
//! | data | the brick blocks, back to back |
//!
//! The apply pass runs three dispatches in order: unlink (node writes that
//! make a record not resident), scatter plus table writes, link (every other
//! node write). Writes to one node or one brick id are merged on the CPU, last
//! write wins, so no index appears twice and the order is only the one
//! decision 0007 describes.

use std::collections::BTreeMap;

use crate::brick_cache::format::{
    BRICK_TABLE_WORDS, BrickRef, GpuNode, INVALID_REF, Kind, NODE_WORDS,
};

pub const HEADER_WORDS: u32 = 16;
pub const NODE_WRITE_WORDS: u32 = 1 + NODE_WORDS;
pub const SCATTER_WORDS: u32 = 4;
pub const TABLE_WRITE_WORDS: u32 = 4;

/// Header word indices, shared with `apply.wesl`.
pub const SHADER_CONSTANTS: &[(&str, u32)] = &[
    ("UPLOAD_HEADER_WORDS", HEADER_WORDS),
    ("UPLOAD_NODE_WRITE_WORDS", NODE_WRITE_WORDS),
    ("UPLOAD_SCATTER_WORDS", SCATTER_WORDS),
    ("UPLOAD_TABLE_WRITE_WORDS", TABLE_WRITE_WORDS),
];

/// A brick block copied from the upload data into the pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scatter {
    /// First word in the batch's data.
    pub src_word: u32,
    pub dst: BrickRef,
    pub words: u32,
}

/// Everything one frame changes on the GPU.
#[derive(Clone, Debug, Default)]
pub struct UploadBatch {
    nodes: BTreeMap<u32, GpuNode>,
    table: BTreeMap<u32, [u32; 2]>,
    scatters: Vec<Scatter>,
    data: Vec<u32>,
}

impl UploadBatch {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.table.is_empty() && self.scatters.is_empty()
    }

    pub fn clear(&mut self) {
        self.nodes.clear();
        self.table.clear();
        self.scatters.clear();
        self.data.clear();
    }

    /// Writes `record` at node `index`, replacing an earlier write.
    pub fn write_node(&mut self, index: u32, record: GpuNode) {
        self.nodes.insert(index, record);
    }

    /// Sets brick table entry `id`.
    pub fn write_table(&mut self, id: u32, block: Option<BrickRef>, last_used: u32) {
        let reference = block.map_or(INVALID_REF, BrickRef::pack);
        self.table.insert(id, [reference, last_used]);
    }

    /// Room for a block in the data, filled by the caller (the encoder
    /// appends to the returned vector). Returns the block's first word.
    pub fn data_mut(&mut self) -> &mut Vec<u32> {
        &mut self.data
    }

    /// Records that `words` words from `src_word` go to `dst`.
    pub fn scatter(&mut self, src_word: u32, dst: BrickRef, words: u32) {
        self.scatters.push(Scatter {
            src_word,
            dst,
            words,
        });
    }

    pub fn node_writes(&self) -> impl Iterator<Item = (u32, GpuNode)> + '_ {
        self.nodes.iter().map(|(&i, &n)| (i, n))
    }

    pub fn scatters(&self) -> &[Scatter] {
        &self.scatters
    }

    /// Bytes of brick data in the batch.
    pub fn data_bytes(&self) -> u64 {
        self.data.len() as u64 * 4
    }

    /// Words the serialized batch takes.
    pub fn serialized_words(&self) -> usize {
        HEADER_WORDS as usize
            + self.nodes.len() * NODE_WRITE_WORDS as usize
            + self.scatters.len() * SCATTER_WORDS as usize
            + self.table.len() * TABLE_WRITE_WORDS as usize
            + self.data.len()
    }

    /// The upload buffer contents, and the entry counts per dispatch.
    pub fn serialize(&self, out: &mut Vec<u32>) -> BatchCounts {
        out.clear();
        out.resize(HEADER_WORDS as usize, 0);
        let (unlink, link): (Vec<_>, Vec<_>) = self
            .nodes
            .iter()
            .partition(|(_, n)| n.kind() == Kind::NotResident);

        let off_unlink = out.len() as u32;
        for &(&index, n) in &unlink {
            out.extend_from_slice(&[index, n.mask_lo, n.mask_hi, n.reference, n.info]);
        }
        let off_scatter = out.len() as u32;
        for s in &self.scatters {
            out.extend_from_slice(&[s.src_word, s.dst.pack(), s.words, 0]);
        }
        let off_link = out.len() as u32;
        for &(&index, n) in &link {
            out.extend_from_slice(&[index, n.mask_lo, n.mask_hi, n.reference, n.info]);
        }
        let off_table = out.len() as u32;
        for (&id, &[reference, last]) in &self.table {
            out.extend_from_slice(&[id, reference, last, 0]);
        }
        let off_data = out.len() as u32;
        out.extend_from_slice(&self.data);
        let counts = BatchCounts {
            unlink: unlink.len() as u32,
            scatter: self.scatters.len() as u32,
            link: link.len() as u32,
            table: self.table.len() as u32,
        };
        let total = out.len() as u32;
        out[..10].copy_from_slice(&[
            counts.unlink,
            counts.scatter,
            counts.link,
            counts.table,
            off_unlink,
            off_scatter,
            off_link,
            off_table,
            off_data,
            total,
        ]);
        counts
    }
}

/// Entries per apply dispatch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BatchCounts {
    pub unlink: u32,
    pub scatter: u32,
    pub link: u32,
    pub table: u32,
}

/// The pools held in CPU memory, updated exactly as the apply pass updates
/// the GPU. Tests use it as the reference the GPU pools must equal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CpuPools {
    pub nodes: Vec<GpuNode>,
    pub segments: Vec<Vec<u32>>,
    pub table: Vec<u32>,
}

impl CpuPools {
    /// Zeroed pools, as wgpu creates them.
    pub fn new(node_count: u32, segment_bytes: &[u64], brick_ids: u32) -> Self {
        Self {
            nodes: vec![GpuNode::default(); node_count as usize],
            segments: segment_bytes
                .iter()
                .map(|&b| vec![0; (b / 4) as usize])
                .collect(),
            table: vec![0; (brick_ids * BRICK_TABLE_WORDS) as usize],
        }
    }

    /// Applies a serialized batch. Out-of-range entries are skipped, as the
    /// GPU's robust buffer access would drop them.
    pub fn apply(&mut self, words: &[u32]) {
        let at = |i: usize| words.get(i).copied().unwrap_or(0) as usize;
        let (n_unlink, n_scatter, n_link, n_table) = (at(0), at(1), at(2), at(3));
        let (off_unlink, off_scatter, off_link, off_table, off_data) =
            (at(4), at(5), at(6), at(7), at(8));
        let node_writes = |pools: &mut Self, off: usize, n: usize| {
            for e in 0..n {
                let b = off + e * NODE_WRITE_WORDS as usize;
                let Some(w) = words.get(b..b + NODE_WRITE_WORDS as usize) else {
                    continue;
                };
                if let Some(node) = pools.nodes.get_mut(w[0] as usize) {
                    *node = GpuNode {
                        mask_lo: w[1],
                        mask_hi: w[2],
                        reference: w[3],
                        info: w[4],
                    };
                }
            }
        };
        node_writes(self, off_unlink, n_unlink);
        for e in 0..n_scatter {
            let b = off_scatter + e * SCATTER_WORDS as usize;
            let (src, dst, n) = (at(b), BrickRef::unpack(at(b + 1) as u32), at(b + 2));
            let Some(seg) = self.segments.get_mut(dst.segment as usize) else {
                continue;
            };
            for j in 0..n {
                if let (Some(&v), Some(slot)) =
                    (words.get(off_data + src + j), seg.get_mut(dst.word() + j))
                {
                    *slot = v;
                }
            }
        }
        for e in 0..n_table {
            let b = off_table + e * TABLE_WRITE_WORDS as usize;
            let id = at(b) * BRICK_TABLE_WORDS as usize;
            if id + 1 < self.table.len() {
                self.table[id] = at(b + 1) as u32;
                self.table[id + 1] = at(b + 2) as u32;
            }
        }
        node_writes(self, off_link, n_link);
    }

    /// The pools as a decoder view.
    pub fn with_view<R>(
        &self,
        f: impl FnOnce(crate::brick_cache::format::decode::PoolView) -> R,
    ) -> R {
        let segments: Vec<&[u32]> = self.segments.iter().map(Vec::as_slice).collect();
        f(crate::brick_cache::format::decode::PoolView {
            nodes: &self.nodes,
            segments: &segments,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use world_core::MaterialId;

    #[test]
    fn serialized_batches_apply_like_the_shader_reads_them() {
        let mut batch = UploadBatch::default();
        let block = BrickRef {
            segment: 1,
            offset16: 2,
        };
        let first = batch.data_mut().len() as u32;
        batch.data_mut().extend_from_slice(&[7, 8, 9, 10]);
        batch.scatter(first, block, 4);
        batch.write_node(3, GpuNode::brick(5, block, MaterialId(1), false));
        batch.write_node(4, GpuNode::not_resident(6, MaterialId(2), false));
        // A later write to the same node wins.
        batch.write_node(3, GpuNode::brick(1, block, MaterialId(1), false));
        batch.write_table(0, Some(block), 12);

        let mut words = Vec::new();
        let counts = batch.serialize(&mut words);
        assert_eq!(
            counts,
            BatchCounts {
                unlink: 1,
                scatter: 1,
                link: 1,
                table: 1
            }
        );
        assert_eq!(words.len(), batch.serialized_words());

        let mut pools = CpuPools::new(8, &[64, 64], 4);
        pools.apply(&words);
        assert_eq!(pools.nodes[3].mask(), 1);
        assert_eq!(pools.nodes[4].kind(), Kind::NotResident);
        assert_eq!(&pools.segments[1][8..12], &[7, 8, 9, 10]);
        assert_eq!(&pools.table[0..2], &[block.pack(), 12]);
    }
}
