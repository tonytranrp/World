//! The 64-tree against the reference models: random single-voxel edits and
//! box fills against the dense array, in a region that crosses the origin and
//! so every node boundary of every level, and sparse edits at the world's
//! faces, edges and corners against a plain map.

use std::collections::BTreeMap;

use proptest::collection::vec;
use proptest::prelude::*;
use world_core::consts::{TREE_DEPTH, WORLD_MIN_VOXEL, WORLD_SIZE_VOXELS};
use world_core::coords::path;
use world_core::tree::NodeRef;
use world_core::{Edit, MaterialId, TreeRead, VoxelBox, VoxelPos, VoxelTree};
use world_testkit::dense::{
    Bounds, DenseWorld, EditOp, NodeCounts, check_canonical, compare_after_ops,
};
use world_testkit::profile::{proptest_config, scale};
use world_testkit::strategies::{edge_pos, edit_ops, material};

/// Straddles the origin, where local coordinates cross `2^19`: a boundary of
/// every node level at once.
const MODEL: Bounds = Bounds::new(VoxelPos::new(-37, -21, -40), [70, 50, 64]);

const WORLD: Bounds = Bounds::new(
    VoxelPos::new(WORLD_MIN_VOXEL, WORLD_MIN_VOXEL, WORLD_MIN_VOXEL),
    [WORLD_SIZE_VOXELS; 3],
);

/// Checks the canonical form and that the walk's counts are the store's.
fn assert_canonical(tree: &VoxelTree) -> Result<(), TestCaseError> {
    let counts = check_canonical(tree).map_err(TestCaseError::fail)?;
    prop_assert_eq!(NodeCounts::from(tree.stats()), counts);
    Ok(())
}

/// Every solid voxel, found by walking the nodes and rebuilding each position
/// from its slot path, independent of `get`.
fn solids(tree: &impl TreeRead) -> Vec<(VoxelPos, MaterialId)> {
    fn walk(
        node: NodeRef<'_>,
        level: u32,
        origin: [u32; 3],
        out: &mut Vec<(VoxelPos, MaterialId)>,
    ) {
        let at =
            |offset: [u32; 3], span: u32| std::array::from_fn(|a| origin[a] + offset[a] * span);
        match node {
            NodeRef::Air | NodeRef::NotResident => {}
            NodeRef::Uniform(m) => {
                let span = path::child_span(level + 1);
                assert!(
                    span <= 16,
                    "a uniform region {span} voxels wide in a sparse world"
                );
                for z in 0..span {
                    for y in 0..span {
                        for x in 0..span {
                            out.push((path::from_world_local(at([x, y, z], 1)), m));
                        }
                    }
                }
            }
            NodeRef::Branch(b) => {
                for (slot, child) in b.children() {
                    let span = path::child_span(level);
                    walk(child, level - 1, at(path::slot_offset(slot), span), out);
                }
            }
            NodeRef::Leaf(l) => {
                for slot in 0..64 {
                    let v = l.voxel(slot);
                    if !v.is_air() {
                        out.push((path::from_world_local(at(path::slot_offset(slot), 1)), v));
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(tree.root(), TREE_DEPTH - 1, [0; 3], &mut out);
    out.sort();
    out
}

proptest! {
    #![proptest_config(proptest_config())]

    #[test]
    fn random_sets_and_fills_match_the_dense_model(
        ops in edit_ops(MODEL, 5, scale().edits_per_case),
    ) {
        let (mut tree, mut dense) = (VoxelTree::new(), DenseWorld::new(MODEL));
        prop_assert_eq!(compare_after_ops(&mut tree, &mut dense, &ops, MODEL), Ok(()));
        assert_canonical(&tree)?;
    }

    #[test]
    fn sparse_edits_at_the_world_edges_match_a_map(
        list in vec((edge_pos(WORLD, 3), material(4)), 0..=scale().edits_per_case),
    ) {
        let mut tree = VoxelTree::new();
        let mut map = BTreeMap::new();
        for &(pos, m) in &list {
            tree.set(pos, m).unwrap();
            if m.is_air() {
                map.remove(&pos);
            } else {
                map.insert(pos, m);
            }
        }
        // Every edited voxel and its neighbours, some of them outside the world.
        for &(pos, _) in &list {
            for d in 0..27 {
                let n = VoxelPos::new(pos.x + d % 3 - 1, pos.y + d / 3 % 3 - 1, pos.z + d / 9 - 1);
                let expected = map.get(&n).copied().unwrap_or(MaterialId::AIR);
                prop_assert_eq!(TreeRead::get(&tree, n), expected, "at {:?}", n);
            }
        }
        let expected: Vec<_> = map.into_iter().collect();
        prop_assert_eq!(solids(&tree), expected);
        assert_canonical(&tree)?;
    }
}

/// A small deterministic generator, so the fixed-budget run needs no proptest.
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u32) -> i32 {
        (self.next() % u64::from(n)) as i32
    }

    fn pos(&mut self, b: Bounds) -> VoxelPos {
        VoxelPos::new(
            b.min.x + self.below(b.size[0]),
            b.min.y + self.below(b.size[1]),
            b.min.z + self.below(b.size[2]),
        )
    }

    /// One in sixteen steps fills a box up to 12 voxels wide; the rest set one voxel.
    fn op(&mut self, b: Bounds) -> EditOp {
        let material = MaterialId((self.next() % 6) as u16);
        if self.next().is_multiple_of(16) {
            let size = [0; 3].map(|_| 1 + self.below(12) as u32);
            let region = VoxelBox::from_min_size(self.pos(b), size).intersection(&b.as_box());
            EditOp::Fill { region, material }
        } else {
            EditOp::Set(Edit::new(self.pos(b), material))
        }
    }
}

/// The plan's done criterion for the voxel core: exactly `cases *
/// edits_per_case` random edits (one million in the full profile) with no
/// divergence from the dense model.
#[test]
fn the_plans_edit_budget_never_diverges() {
    let s = scale();
    let mut rng = XorShift(0x2545_F491_4F6C_DD1D);
    let mut applied = 0;
    for case in 0..s.cases {
        let (mut tree, mut dense) = (VoxelTree::new(), DenseWorld::new(MODEL));
        let ops: Vec<EditOp> = (0..s.edits_per_case).map(|_| rng.op(MODEL)).collect();
        assert_eq!(
            compare_after_ops(&mut tree, &mut dense, &ops, MODEL),
            Ok(()),
            "case {case}"
        );
        applied += ops.len();
    }
    assert_eq!(applied, s.cases as usize * s.edits_per_case);
}

#[test]
fn a_new_tree_is_air_everywhere() {
    let tree = VoxelTree::new();
    assert!(matches!(tree.root(), NodeRef::Air));
    assert_eq!(NodeCounts::from(tree.stats()), NodeCounts::default());
    for p in [VoxelPos::ORIGIN, WORLD.min, VoxelPos::new(-1, -1, -1)] {
        assert_eq!(TreeRead::get(&tree, p), MaterialId::AIR);
    }
}

#[test]
fn reads_outside_the_world_are_air_even_beside_solid_voxels() {
    let mut tree = VoxelTree::new();
    tree.fill_box(VoxelBox::WORLD, MaterialId(3)).unwrap();
    let (lo, hi) = (VoxelBox::WORLD.min(), VoxelBox::WORLD.end());
    for p in [
        VoxelPos::new(lo.x - 1, 0, 0),
        VoxelPos::new(0, hi.y, 0),
        VoxelPos::new(0, 0, i32::MIN),
        VoxelPos::new(i32::MAX, i32::MAX, i32::MAX),
    ] {
        assert_eq!(TreeRead::get(&tree, p), MaterialId::AIR, "{p:?}");
    }
    assert_eq!(TreeRead::get(&tree, lo), MaterialId(3));
}
