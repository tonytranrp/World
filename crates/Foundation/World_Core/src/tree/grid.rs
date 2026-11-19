//! Dense small grids: the voxel storage of dynamic objects, read through the
//! same [`VoxelSource`] interface as the world tree.

use std::fmt;

use super::node::path_key;
use super::read::{NodeKind, NodeView, VoxelSource};
use crate::consts::TREE_FANOUT;
use crate::coords::{VoxelBox, VoxelPos};
use crate::material::MaterialId;

/// The most levels a grid may have: 1024 voxels per axis.
pub const MAX_GRID_LEVELS: u32 = 5;

/// A cube of `4^levels` voxels per axis at an origin, every voxel stored, as
/// the plan gives dynamic objects (`levels` is 1 to [`MAX_GRID_LEVELS`]).
///
/// Writes are O(1) plus a refresh of the per-node summaries on the voxel's
/// path, which stops at the first node that did not change. Reads go through
/// [`VoxelSource`] with canonical views: each node's occupancy mask and
/// whether it is all one material are kept up to date, so [`NodeView::mask`]
/// and [`NodeView::kind`] are lookups and traversal skips empty and uniform
/// space exactly as it does in the world tree.
///
/// A grid holds `64^levels` two-byte voxels: 128 KiB at 3 levels (64 voxels
/// per axis, 1 m), 8 MiB at 4.
#[derive(Clone, PartialEq, Eq)]
pub struct DenseGrid {
    levels: u32,
    origin: VoxelPos,
    /// Every voxel in tree order: the voxel whose path key is `k` is
    /// `voxels[k]`, so leaf `n` holds `voxels[64n..64n + 64]` in slot order.
    voxels: Vec<MaterialId>,
    /// `nodes[level][n]` summarizes node `n` of `level`. The children of node
    /// `n` at a level above 0 are nodes `64n..64n + 64` one level down.
    nodes: Vec<Vec<Summary>>,
}

/// What a node holds, kept so views never scan voxels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Summary {
    /// Bit `i` is set when child (or voxel) `i` is not all air.
    mask: u64,
    /// The one material of every voxel, air included, when there is one.
    fill: Option<MaterialId>,
}

impl Summary {
    const AIR: Self = Self {
        mask: 0,
        fill: Some(MaterialId::AIR),
    };

    /// The summary of a leaf from its 64 voxels.
    fn of_voxels(voxels: &[MaterialId]) -> Self {
        let mask = (0..)
            .zip(voxels)
            .filter(|(_, m)| !m.is_air())
            .fold(0, |mask, (slot, _)| mask | 1u64 << slot);
        let first = voxels[0];
        let fill = voxels.iter().all(|&m| m == first).then_some(first);
        Self { mask, fill }
    }

    /// The summary of a node from its 64 children's summaries.
    fn of_children(children: &[Self]) -> Self {
        let mask = (0..)
            .zip(children)
            .filter(|(_, c)| c.fill != Some(MaterialId::AIR))
            .fold(0, |mask, (slot, _)| mask | 1u64 << slot);
        let first = children[0].fill;
        let fill = first.filter(|_| children.iter().all(|c| c.fill == first));
        Self { mask, fill }
    }
}

impl DenseGrid {
    /// An all-air grid of `levels` levels whose lowest voxel is `origin`.
    /// Panics if `levels` is not 1 to [`MAX_GRID_LEVELS`] or the grid would
    /// reach past `i32` coordinates.
    pub fn new(levels: u32, origin: VoxelPos) -> Self {
        assert!(
            (1..=MAX_GRID_LEVELS).contains(&levels),
            "a grid has 1 to {MAX_GRID_LEVELS} levels, not {levels}"
        );
        let last = i64::from(1u32 << (2 * levels)) - 1;
        assert!(
            origin
                .to_array()
                .iter()
                .all(|&o| i64::from(o) + last <= i64::from(i32::MAX)),
            "a grid at {origin:?} reaches past i32 coordinates"
        );
        Self {
            levels,
            origin,
            voxels: vec![MaterialId::AIR; 1 << (6 * levels)],
            nodes: (0..levels)
                .map(|level| vec![Summary::AIR; 1 << (6 * (levels - 1 - level))])
                .collect(),
        }
    }

    /// Voxels per axis: `4^levels`.
    pub fn size(&self) -> u32 {
        1 << (2 * self.levels)
    }

    /// The voxels the grid covers.
    pub fn bounds(&self) -> VoxelBox {
        VoxelBox::from_min_size(self.origin, [self.size(); 3])
    }

    /// The material of one voxel; air outside the grid.
    pub fn get(&self, pos: VoxelPos) -> MaterialId {
        match self.local(pos) {
            Some(local) => self.voxels[path_key(local, self.levels) as usize],
            None => MaterialId::AIR,
        }
    }

    /// Sets one voxel. Returns whether its material changed. Panics if `pos`
    /// is outside the grid.
    pub fn set(&mut self, pos: VoxelPos, material: MaterialId) -> bool {
        let Some(local) = self.local(pos) else {
            panic!("voxel {pos:?} is outside the grid {:?}", self.bounds());
        };
        let key = path_key(local, self.levels) as usize;
        if self.voxels[key] == material {
            return false;
        }
        self.voxels[key] = material;
        // Refresh the summaries up the path; an unchanged one leaves every
        // ancestor unchanged too.
        let mut n = key / 64;
        for level in 0..self.levels as usize {
            let children = 64 * n..64 * n + 64;
            let summary = if level == 0 {
                Summary::of_voxels(&self.voxels[children])
            } else {
                Summary::of_children(&self.nodes[level - 1][children])
            };
            if self.nodes[level][n] == summary {
                break;
            }
            self.nodes[level][n] = summary;
            n /= 64;
        }
        true
    }

    /// The grid-local coordinate of `pos`, or `None` outside the grid.
    fn local(&self, pos: VoxelPos) -> Option<[u32; 3]> {
        let size = i64::from(self.size());
        let axis = |p: i32, o: i32| {
            let d = i64::from(p) - i64::from(o);
            (0..size).contains(&d).then_some(d as u32)
        };
        Some([
            axis(pos.x, self.origin.x)?,
            axis(pos.y, self.origin.y)?,
            axis(pos.z, self.origin.z)?,
        ])
    }
}

impl VoxelSource for DenseGrid {
    type Node<'a> = GridNode<'a>;

    fn levels(&self) -> u32 {
        self.levels
    }

    fn origin(&self) -> VoxelPos {
        self.origin
    }

    fn root(&self) -> GridNode<'_> {
        GridNode {
            grid: self,
            level: self.levels - 1,
            index: 0,
        }
    }

    fn get(&self, pos: VoxelPos) -> MaterialId {
        DenseGrid::get(self, pos)
    }
}

impl fmt::Debug for DenseGrid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DenseGrid")
            .field("levels", &self.levels)
            .field("origin", &self.origin)
            .field("root", &self.nodes[self.levels as usize - 1][0])
            .finish_non_exhaustive()
    }
}

/// One node of a [`DenseGrid`], as a [`NodeView`].
#[derive(Clone, Copy)]
pub struct GridNode<'a> {
    grid: &'a DenseGrid,
    level: u32,
    index: usize,
}

impl GridNode<'_> {
    fn summary(&self) -> Summary {
        self.grid.nodes[self.level as usize][self.index]
    }
}

impl NodeView for GridNode<'_> {
    fn kind(&self) -> NodeKind {
        match self.summary().fill {
            Some(m) if m.is_air() => NodeKind::Air,
            Some(m) => NodeKind::Uniform(m),
            None if self.level == 0 => NodeKind::Leaf,
            None => NodeKind::Branch,
        }
    }

    fn mask(&self) -> u64 {
        self.summary().mask
    }

    fn child(&self, slot: u32) -> Self {
        assert!(slot < TREE_FANOUT, "child slot {slot} is above 63");
        if self.level == 0 {
            assert!(
                self.summary().fill.is_some(),
                "a leaf has voxels, not children"
            );
            return *self;
        }
        Self {
            grid: self.grid,
            level: self.level - 1,
            index: 64 * self.index + slot as usize,
        }
    }

    fn voxel(&self, slot: u32) -> MaterialId {
        assert!(slot < TREE_FANOUT, "voxel slot {slot} is above 63");
        if let Some(m) = self.summary().fill {
            return m;
        }
        assert!(self.level == 0, "a branch has children, not voxels");
        self.grid.voxels[64 * self.index + slot as usize]
    }
}

impl fmt::Debug for GridNode<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GridNode")
            .field("level", &self.level)
            .field("index", &self.index)
            .field("summary", &self.summary())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::node::slot_at;

    const STONE: MaterialId = MaterialId(1);
    const DIRT: MaterialId = MaterialId(2);

    fn grid(levels: u32) -> DenseGrid {
        DenseGrid::new(levels, VoxelPos::new(-10, 3, 100))
    }

    fn at(g: &DenseGrid, x: i32, y: i32, z: i32) -> VoxelPos {
        let o = g.origin;
        VoxelPos::new(o.x + x, o.y + y, o.z + z)
    }

    #[test]
    fn a_new_grid_is_air_with_an_air_root() {
        let g = grid(2);
        assert_eq!((g.size(), g.levels()), (16, 2));
        assert_eq!(g.bounds().min(), g.origin);
        assert_eq!(g.root().kind(), NodeKind::Air);
        assert_eq!(g.root().mask(), 0);
        assert_eq!(g.get(at(&g, 3, 3, 3)), MaterialId::AIR);
    }

    #[test]
    fn voxels_are_stored_in_tree_order() {
        let mut g = grid(2);
        let p = at(&g, 5, 2, 9);
        assert!(g.set(p, STONE));
        assert!(!g.set(p, STONE));
        let key = path_key([5, 2, 9], 2) as usize;
        assert_eq!(g.voxels[key], STONE);
        assert_eq!(key % 64, slot_at([5, 2, 9], 0) as usize);
        assert_eq!(g.get(p), STONE);
        assert_eq!(VoxelSource::get(&g, p), STONE);
        assert_eq!(g.get(at(&g, 5, 2, 8)), MaterialId::AIR);
        assert_eq!(g.get(at(&g, 16, 0, 0)), MaterialId::AIR, "outside");
        assert_eq!(g.get(at(&g, -1, 0, 0)), MaterialId::AIR, "outside");
    }

    #[test]
    fn summaries_follow_writes_both_ways() {
        let mut g = grid(2);
        // Fill the leaf at slot 0 of the root with stone, voxel by voxel.
        for i in 0..64 {
            g.set(at(&g, i % 4, i / 4 % 4, i / 16), STONE);
            let leaf = g.root().child(0);
            let expected = if i == 63 {
                NodeKind::Uniform(STONE)
            } else {
                NodeKind::Leaf
            };
            assert_eq!(leaf.kind(), expected, "after {i}");
            assert_eq!(leaf.mask(), u64::MAX >> (63 - i));
        }
        assert_eq!(g.root().kind(), NodeKind::Branch);
        assert_eq!(g.root().mask(), 1);
        assert_eq!(g.root().child(0).child(7).voxel(9), STONE);

        // One voxel of dirt splits the uniform leaf again.
        g.set(at(&g, 1, 0, 0), DIRT);
        let leaf = g.root().child(0);
        assert_eq!(leaf.kind(), NodeKind::Leaf);
        assert_eq!(leaf.voxel(1), DIRT);
        assert_eq!(leaf.palette().as_slice(), [STONE, DIRT]);
        // Clearing it all returns the grid to air.
        for i in 0..64 {
            g.set(at(&g, i % 4, i / 4 % 4, i / 16), MaterialId::AIR);
        }
        assert_eq!(g, grid(2));
        assert_eq!(g.root().kind(), NodeKind::Air);
    }

    #[test]
    fn a_whole_grid_of_one_material_is_a_uniform_root() {
        let mut g = grid(1);
        for i in 0..64 {
            g.set(at(&g, i % 4, i / 4 % 4, i / 16), DIRT);
        }
        assert_eq!(g.root().kind(), NodeKind::Uniform(DIRT));
        assert_eq!(g.root().child(5).kind(), NodeKind::Uniform(DIRT));
        assert_eq!(g.root().voxel(5), DIRT);
    }

    #[test]
    #[should_panic(expected = "outside the grid")]
    fn writes_outside_the_grid_panic() {
        let mut g = grid(1);
        let p = at(&g, 4, 0, 0);
        g.set(p, STONE);
    }

    #[test]
    #[should_panic(expected = "1 to 5 levels")]
    fn levels_are_bounded() {
        DenseGrid::new(6, VoxelPos::ORIGIN);
    }

    #[test]
    #[should_panic(expected = "past i32")]
    fn the_grid_must_fit_in_coordinates() {
        DenseGrid::new(2, VoxelPos::new(i32::MAX - 10, 0, 0));
    }
}
