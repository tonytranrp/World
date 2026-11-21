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
    /// position that is not a chunk's. Costs what summarizing the chunk on
    /// the render thread would: about a microsecond per mixed node.
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
/// under the level-4 node at the origin (a part only keeps paths), inserted
/// in slot order with children before their parent.
fn chunk_part<N: NodeView>(chunk: N) -> Part<Summary> {
    let top = NodeKey {
        level: WARMED_LEVEL as u8,
        x: 0,
        y: 0,
        z: 0,
    };
    let mut part = Part::new(WARMED_LEVEL);
    let mut tally = Tally::default();
    for (s1, child) in chunk.children() {
        if content(&child) != Content::Branch {
            continue;
        }
        let k3 = top.child(s1);
        let mut node = Tally::default();
        for (s2, grandchild) in child.children() {
            let s = match content(&grandchild) {
                Content::Branch => {
                    summarize_level2_node(grandchild, k3.child(s2), &mut tally, &mut |k, s| {
                        part.insert(k, s);
                    })
                }
                kind => plain_summary(grandchild, kind, 2),
            };
            node.add(s.material, s.solid);
        }
        part.insert(k3, node.summary());
    }
    part
}
