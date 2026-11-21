//! A chunk's summaries, computed where the chunk is built (decision 0022).
//!
//! The summaries of a replaced chunk's nodes (decision 0012) depend only on
//! its content, so whoever builds or loads the chunk off the frame can
//! compute them and hand them to the renderer with the chunk change. The
//! render thread then inserts them in one step instead of computing
//! thousands a frame. They carry the content they describe, and are used
//! only when the renderer's source holds exactly that content
//! ([`ChunkData::same_arenas`]); anything else (an edit since, a newer
//! replacement) falls back to computing them on the render thread.

use world_core::tree::{CHUNK_LEVEL, ChunkData, NodeView, VoxelSource};
use world_core::{VoxelPos, VoxelTree};

use super::key::{NodeKey, Shape, view_at};
use super::table::Part;
use crate::brick_cache::format::summary::{
    Content, Summary, Tally, content, count_voxels, exact_summary, node_volume,
};

/// The level whose nodes the renderer summarizes a grandchild at a time:
/// their grandchildren are level-2 nodes.
pub(super) const WARMED_LEVEL: u32 = 4;

const _: () = assert!(CHUNK_LEVEL == WARMED_LEVEL, "chunks are level-4 nodes");

/// The summaries of every mixed node below one chunk (levels 1 to 3), as
/// the renderer would compute them on applying the chunk, with the content
/// they were computed from.
#[derive(Clone)]
pub struct ChunkSummaries {
    min: VoxelPos,
    data: ChunkData,
    part: Part<Summary>,
}

impl std::fmt::Debug for ChunkSummaries {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChunkSummaries")
            .field("min", &self.min)
            .field("nodes", &self.part.len())
            .finish_non_exhaustive()
    }
}

impl ChunkSummaries {
    /// The summaries of `data`, the content of the chunk whose lowest voxel
    /// is `min`. `None` for air or one material, which have none, or a
    /// position that is not a chunk's. Bricks are counted from what the tree
    /// stores per brick ([`NodeView::brick_counts`]), not leaf by leaf, and
    /// the part is built with each array allocated once (decision 0024).
    pub fn compute(min: VoxelPos, data: &ChunkData) -> Option<Self> {
        if data.uniform().is_some() {
            return None;
        }
        let mut tree = VoxelTree::unloaded();
        tree.insert_chunk(min, data.clone()).ok()?;
        let local = Shape::of(&tree).local(min)?;
        let key = NodeKey::containing(local, CHUNK_LEVEL);
        let view = view_at(&tree, key);
        (content(&view) == Content::Branch).then(|| Self {
            min,
            data: data.clone(),
            part: chunk_part(view),
        })
    }

    /// The chunk's lowest voxel.
    pub fn min(&self) -> VoxelPos {
        self.min
    }

    /// Nodes summarized.
    pub fn len(&self) -> usize {
        self.part.len()
    }

    pub fn is_empty(&self) -> bool {
        self.part.is_empty()
    }

    /// Whether `source` holds at the chunk exactly the content these
    /// summaries were computed from. O(1); never true for sources that do
    /// not store chunks as arenas.
    pub fn describes(&self, source: &impl VoxelSource) -> bool {
        source
            .shared_chunk(self.min)
            .is_some_and(|now| self.data.same_arenas(&now))
    }

    /// The summary of a node below the chunk (any key at the chunk level
    /// above it names the chunk), if it is mixed.
    pub fn get(&self, key: &NodeKey) -> Option<Summary> {
        self.part.get(key).copied()
    }

    pub(super) fn into_part(self) -> Part<Summary> {
        self.part
    }
}

/// The summary of a child that is not mixed, as
/// [`Residency::summary`](super::residency::Residency) answers it without
/// the cache: air or not loaded is nothing, one material weighs the child's
/// volume, and a leaf counts its voxels.
pub(super) fn plain_summary<N: NodeView>(child: N, kind: Content, level: u32) -> Summary {
    match kind {
        Content::Air | Content::Unloaded => Summary::default(),
        Content::Uniform(m) => Summary {
            material: m,
            solid: node_volume(level),
        },
        Content::Leaf | Content::Branch => exact_summary(child, 0),
    }
}

/// The summary of the mixed level-2 node `view` (key `key`), after those of
/// its mixed children, each passed to `out` as it is computed, the node's
/// own last. `tally` is scratch.
pub(super) fn summarize_level2_node<N: NodeView>(
    view: N,
    key: NodeKey,
    tally: &mut Tally,
    out: &mut impl FnMut(NodeKey, Summary),
) -> Summary {
    debug_assert_eq!(key.level, 2, "level-2 nodes weigh level-1 children");
    let mut node = Tally::default();
    for (slot, child) in view.children() {
        let s = match content(&child) {
            Content::Branch => {
                tally.clear();
                count_voxels(child, 1, tally);
                let s = tally.summary();
                out(key.child(slot), s);
                s
            }
            kind => plain_summary(child, kind, 1),
        };
        node.add(s.material, s.solid);
    }
    let s = node.summary();
    out(key, s);
    s
}

/// The summaries of every mixed node below the level-4 node `chunk`, keyed
/// under the level-4 node at the origin (a part only keeps paths), computed
/// in slot order with children before their parent and stored with each
/// array allocated once.
fn chunk_part<N: NodeView>(chunk: N) -> Part<Summary> {
    let mut entries = Vec::new();
    summarize_chunk(chunk, &mut |k, s| entries.push((k, s)));
    Part::from_grouped(WARMED_LEVEL, &entries)
}

/// [`chunk_part`]'s summaries, each passed to `out` as it is computed.
fn summarize_chunk<N: NodeView>(chunk: N, out: &mut impl FnMut(NodeKey, Summary)) {
    let top = NodeKey {
        level: WARMED_LEVEL as u8,
        x: 0,
        y: 0,
        z: 0,
    };
    let mut tally = Tally::default();
    for (s1, child) in chunk.children() {
        if content(&child) != Content::Branch {
            continue;
        }
        let k3 = top.child(s1);
        let mut node = Tally::default();
        for (s2, grandchild) in child.children() {
            let s = match content(&grandchild) {
                Content::Branch => summarize_level2_node(grandchild, k3.child(s2), &mut tally, out),
                kind => plain_summary(grandchild, kind, 2),
            };
            node.add(s.material, s.solid);
        }
        out(k3, node.summary());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use world_core::tree::{NodeKind, RegionSource};
    use world_gen::{ChunkCoord, CoarseSource, Generator, GeneratorVersion, TerrainConfig};

    /// A view that hides how its source stores bricks: every brick is
    /// counted leaf by leaf, and every leaf of two or more materials voxel
    /// by voxel.
    #[derive(Clone, Copy)]
    struct Plain<N>(N);

    impl<N: NodeView> NodeView for Plain<N> {
        fn kind(&self) -> NodeKind {
            self.0.kind()
        }
        fn mask(&self) -> u64 {
            self.0.mask()
        }
        fn child(&self, slot: u32) -> Self {
            Plain(self.0.child(slot))
        }
        fn voxel(&self, slot: u32) -> world_core::MaterialId {
            self.0.voxel(slot)
        }
    }

    /// Every summary below the chunk-level view `chunk`, in order.
    fn summaries<N: NodeView>(chunk: N) -> Vec<(NodeKey, Summary)> {
        let mut out = Vec::new();
        summarize_chunk(chunk, &mut |k, s| out.push((k, s)));
        out
    }

    /// On real terrain (a surface chunk, the chunk below it, and the
    /// surface chunk built from coarse cells), the summaries counted from
    /// the tree's stored bricks are exactly those counted voxel by voxel.
    #[test]
    fn stored_brick_counts_give_the_summaries_of_counting_every_voxel() {
        let generator = Generator::new(0x5EED, GeneratorVersion::CURRENT, TerrainConfig::default())
            .expect("the default config is valid");
        let probe = ChunkCoord::new(512, 0, 512).unwrap().min_voxel();
        let (x, z) = (probe.x + 512, probe.z + 512);
        let height = generator.column(x, z).unwrap().height;
        let top = ChunkCoord::containing(VoxelPos::new(x, height, z)).unwrap();
        let below = ChunkCoord::new(top.x(), top.y() - 1, top.z()).unwrap();
        let coarse = CoarseSource::new(generator.coarse_columns(top, 2).unwrap());
        let cases: [(ChunkCoord, &dyn Fn(&mut VoxelTree, VoxelPos)); 3] = [
            (top, &|t, min| build(t, min, &generator.chunk_source(top))),
            (below, &|t, min| {
                build(t, min, &generator.chunk_source(below))
            }),
            (top, &|t, min| build(t, min, &coarse)),
        ];
        for (coord, make) in cases {
            let min = coord.min_voxel();
            let mut tree = VoxelTree::new();
            make(&mut tree, min);
            let key = NodeKey::containing(Shape::of(&tree).local(min).unwrap(), CHUNK_LEVEL);
            let view = view_at(&tree, key);
            let fast = summaries(view);
            assert!(fast.len() > 100, "{} mixed nodes", fast.len());
            assert_eq!(fast, summaries(Plain(view)));
            let offer = ChunkSummaries::compute(min, &tree.chunk_data(min).unwrap()).unwrap();
            assert_eq!(offer.len(), fast.len());
            for (k, s) in fast {
                assert_eq!(offer.get(&k), Some(s), "{k:?}");
            }
        }
    }

    fn build(tree: &mut VoxelTree, min: VoxelPos, source: &impl RegionSource) {
        tree.build_region(CHUNK_LEVEL, min, source).unwrap();
    }
}
