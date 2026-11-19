//! White-box tests of the whole tree: canonical form, storage accounting,
//! snapshots and the edit entry points, plus a property test against a dense
//! model. The black-box suite lives in the crate's `tests/` folder.

use proptest::collection::vec;
use proptest::prelude::*;

use super::node::leaf::{index_words, palette_words, width_for};
use super::*;
use crate::BrickPos;
use crate::consts::{WORLD_END_VOXEL, WORLD_MIN_VOXEL};

const STONE: MaterialId = MaterialId(1);
const DIRT: MaterialId = MaterialId(2);

fn pos(x: i32, y: i32, z: i32) -> VoxelPos {
    VoxelPos::new(x, y, z)
}

fn cube(min: [i32; 3], size: u32) -> VoxelBox {
    VoxelBox::from_min_size(min.into(), [size; 3])
}

/// The content-dependent part of the stats: branches, leaves, words.
fn live(tree: &VoxelTree) -> (usize, usize, usize) {
    let s = tree.stats();
    (s.branches, s.leaves, s.words)
}

/// Walks the tree, asserting every canonical-form rule, and returns the live
/// (branches, leaves, words) it found, to compare with the stats.
fn assert_canonical(tree: &impl TreeRead) -> (usize, usize, usize) {
    fn walk(node: NodeRef<'_>, level: u32, found: &mut (usize, usize, usize)) {
        match node {
            NodeRef::Air => {}
            NodeRef::Uniform(m) => assert!(!m.is_air(), "a uniform child is never air"),
            NodeRef::Branch(b) => {
                assert_eq!(b.level(), level);
                assert_ne!(b.mask(), 0, "an all-air branch must be a clear bit");
                let children: Vec<_> = b.children().collect();
                assert_eq!(children.len(), b.mask().count_ones() as usize);
                let one_material = match children[0].1 {
                    NodeRef::Uniform(m) => children
                        .iter()
                        .all(|(_, c)| matches!(c, NodeRef::Uniform(x) if *x == m)),
                    _ => false,
                };
                assert!(
                    !(b.mask() == u64::MAX && one_material),
                    "a branch of one material must be a uniform child"
                );
                found.0 += 1;
                found.2 += children.len();
                for (slot, child) in children {
                    assert!(b.mask() & (1 << slot) != 0);
                    assert!(!matches!(child, NodeRef::Air));
                    walk(child, level - 1, found);
                }
            }
            NodeRef::Leaf(l) => {
                assert_eq!(level, 0, "leaves sit at level 0");
                assert_ne!(l.mask(), 0, "an all-air leaf must be a clear bit");
                let palette: Vec<_> = l.palette().collect();
                assert!(palette.windows(2).all(|w| w[0] < w[1]), "sorted, unique");
                assert!(!palette.contains(&MaterialId::AIR));
                let voxels: Vec<_> = (0..64).map(|s| l.voxel(s)).collect();
                for m in &palette {
                    assert!(voxels.contains(m), "palette entry {m:?} is unused");
                }
                assert!(
                    !(l.mask() == u64::MAX && palette.len() == 1),
                    "a full leaf of one material must be a uniform child"
                );
                assert_eq!(l.index_bits(), width_for(palette.len()), "minimal width");
                let solid = l.mask().count_ones() as usize;
                let indices = index_words(solid, l.index_bits());
                assert_eq!(l.index_words().len(), indices);
                found.1 += 1;
                found.2 += palette_words(palette.len()) + indices;
            }
        }
    }
    let mut found = (0, 0, 0);
    walk(tree.root(), ROOT_LEVEL, &mut found);
    found
}

/// Asserts canonical form and that the stats count exactly what is reachable.
fn assert_sound(tree: &VoxelTree) {
    assert_eq!(
        assert_canonical(tree),
        live(tree),
        "stats disagree with the tree"
    );
}

#[test]
fn a_new_tree_is_air_and_holds_no_storage() {
    let tree = VoxelTree::new();
    assert!(matches!(tree.root(), NodeRef::Air));
    assert_eq!(tree.stats(), TreeStats::default());
    assert_eq!(tree.get(pos(0, 0, 0)), MaterialId::AIR);
}

#[test]
fn set_reports_whether_the_voxel_changed() {
    let mut tree = VoxelTree::new();
    let p = pos(-3, 7, 100);
    assert_eq!(tree.set(p, STONE), Ok(true));
    assert_eq!(tree.set(p, STONE), Ok(false));
    assert_eq!(tree.get(p), STONE);
    assert_eq!(tree.get(pos(-3, 7, 101)), MaterialId::AIR);
    assert_eq!(tree.set(p, DIRT), Ok(true));
    assert_eq!(tree.get(p), DIRT);
    assert_sound(&tree);
}

#[test]
fn the_world_edges_are_writable_and_the_outside_is_not() {
    let (lo, hi) = (WORLD_MIN_VOXEL, WORLD_END_VOXEL - 1);
    let mut tree = VoxelTree::new();
    let corners: Vec<VoxelPos> = (0..8)
        .map(|i| {
            let pick = |bit: i32| if i & bit == 0 { lo } else { hi };
            pos(pick(1), pick(2), pick(4))
        })
        .collect();
    for (i, &c) in corners.iter().enumerate() {
        assert_eq!(tree.set(c, MaterialId(i as u16 + 1)), Ok(true));
    }
    for (i, &c) in corners.iter().enumerate() {
        assert_eq!(tree.get(c), MaterialId(i as u16 + 1));
    }
    assert_eq!(tree.stats().leaves, 8);
    assert_sound(&tree);

    for outside in [pos(lo - 1, 0, 0), pos(0, hi + 1, 0), pos(0, 0, i32::MIN)] {
        assert_eq!(
            tree.set(outside, STONE),
            Err(EditError::OutOfWorld { pos: outside })
        );
        assert_eq!(tree.get(outside), MaterialId::AIR);
    }
}

#[test]
fn refused_edits_change_nothing_and_copy_nothing() {
    let mut tree = VoxelTree::new();
    tree.fill_box(cube([0, 0, 0], 5), STONE).unwrap();
    let before = tree.snapshot();
    let stats = tree.stats();
    let outside = pos(WORLD_END_VOXEL, 0, 0);

    assert!(tree.set(outside, DIRT).is_err());
    let straddling = VoxelBox::new(pos(0, 0, 0), pos(WORLD_END_VOXEL + 5, 1, 1));
    assert_eq!(
        tree.fill_box(straddling, DIRT),
        Err(EditError::OutOfWorld {
            pos: pos(WORLD_END_VOXEL + 4, 0, 0)
        })
    );
    let batch = [Edit::new(pos(1, 1, 1), DIRT), Edit::new(outside, DIRT)];
    assert_eq!(
        tree.apply(&batch),
        Err(EditError::OutOfWorld { pos: outside })
    );

    assert_eq!(tree.get(pos(1, 1, 1)), STONE, "the batch applied nothing");
    assert_eq!(tree.stats(), stats);
    assert!(tree.store.storage().shares_everything_with(&before.storage));
}

#[test]
fn writes_that_change_nothing_copy_and_report_nothing() {
    let mut tree = VoxelTree::new();
    tree.fill_box(cube([-8, -8, -8], 16), STONE).unwrap();
    tree.set(pos(0, 0, 0), DIRT).unwrap();
    let before = tree.snapshot();
    let stats = tree.stats();

    assert_eq!(tree.set(pos(0, 0, 0), DIRT), Ok(false));
    assert_eq!(tree.set(pos(1, 0, 0), STONE), Ok(false));
    assert_eq!(tree.set(pos(500, 0, 0), MaterialId::AIR), Ok(false));
    assert_eq!(tree.fill_box(cube([-8, -8, -8], 8), STONE), Ok(false));
    assert_eq!(
        tree.fill_box(cube([100, 0, 0], 50), MaterialId::AIR),
        Ok(false)
    );
    assert_eq!(tree.fill_box(cube([0, 0, 0], 0), DIRT), Ok(false));
    let no_ops = [
        Edit::new(pos(0, 0, 0), DIRT),
        Edit::new(pos(-8, -8, -8), STONE),
    ];
    assert!(tree.apply(&no_ops).unwrap().is_empty());

    assert_eq!(tree.stats(), stats);
    assert!(tree.store.storage().shares_everything_with(&before.storage));
}

#[test]
fn snapshots_never_see_later_writes() {
    let mut tree = VoxelTree::new();
    tree.fill_box(cube([0, 0, 0], 20), STONE).unwrap();
    let snapshot = tree.snapshot();
    let copy = snapshot.clone();

    tree.set(pos(3, 3, 3), DIRT).unwrap();
    tree.fill_box(cube([0, 0, 0], 4), MaterialId::AIR).unwrap();
    tree.fill_box(VoxelBox::WORLD, DIRT).unwrap();

    let reader = std::thread::spawn(move || {
        (
            snapshot.get(pos(3, 3, 3)),
            snapshot.get(pos(0, 0, 0)),
            snapshot.get(pos(25, 0, 0)),
        )
    });
    assert_eq!(reader.join().unwrap(), (STONE, STONE, MaterialId::AIR));
    assert_eq!(copy.get(pos(19, 19, 19)), STONE);
    assert_canonical(&copy);
    assert!(matches!(tree.root(), NodeRef::Uniform(DIRT)));
    assert_eq!(live(&tree), (0, 0, 0));
}

#[test]
fn snapshots_compare_by_content() {
    let mut a = VoxelTree::new();
    let mut b = VoxelTree::new();
    a.set(pos(1, 2, 3), STONE).unwrap();
    b.fill_box(cube([0, 0, 0], 8), STONE).unwrap();
    b.fill_box(cube([0, 0, 0], 8), MaterialId::AIR).unwrap();
    b.set(pos(1, 2, 3), STONE).unwrap();
    assert_eq!(a, b);
    assert_eq!(a.snapshot(), b.snapshot());
    b.set(pos(1, 2, 3), DIRT).unwrap();
    assert_ne!(a, b);
}

#[test]
fn clearing_a_region_returns_every_node_and_word() {
    let mut tree = VoxelTree::new();
    for i in 0..300 {
        let p = pos((i * 7) % 37 - 18, (i * 11) % 29 - 14, (i * 13) % 31 - 15);
        tree.set(p, MaterialId(1 + (i % 20) as u16)).unwrap();
    }
    tree.fill_box(cube([-30, -30, -30], 10), DIRT).unwrap();
    assert_sound(&tree);
    let grown = tree.stats();
    assert!(grown.leaves > 0 && grown.branches > 0);

    tree.fill_box(cube([-40, -40, -40], 80), MaterialId::AIR)
        .unwrap();
    assert!(matches!(tree.root(), NodeRef::Air));
    assert_eq!(live(&tree), (0, 0, 0));
    let cleared = tree.stats();
    assert_eq!(
        cleared.free_nodes,
        grown.branches + grown.leaves + grown.free_nodes
    );
    assert_eq!(cleared.bytes, grown.bytes, "freeing keeps the pages");
}

#[test]
fn clearing_voxel_by_voxel_also_returns_everything() {
    let mut tree = VoxelTree::new();
    let points: Vec<VoxelPos> = (0..200)
        .map(|i| pos(i % 9 * 5 - 20, i % 7 * 6 - 20, i % 5 * 17 - 40))
        .collect();
    for (i, &p) in points.iter().enumerate() {
        tree.set(p, MaterialId(1 + i as u16 % 30)).unwrap();
    }
    assert_sound(&tree);
    for &p in &points {
        tree.set(p, MaterialId::AIR).unwrap();
        assert_sound(&tree);
    }
    assert_eq!(live(&tree), (0, 0, 0));
}

#[test]
fn freed_storage_is_reused_before_the_arena_grows() {
    let mut tree = VoxelTree::new();
    let region = cube([-10, -10, -10], 21);
    let build = |tree: &mut VoxelTree| {
        for i in 0..400 {
            let p = pos(i % 21 - 10, (i / 21) % 21 - 10, (i * 5) % 21 - 10);
            tree.set(p, MaterialId(1 + (i % 7) as u16)).unwrap();
        }
    };
    build(&mut tree);
    let first = tree.stats();
    tree.fill_box(region, MaterialId::AIR).unwrap();
    build(&mut tree);
    let second = tree.stats();
    assert_eq!(live(&tree), (first.branches, first.leaves, first.words));
    assert_eq!(second.bytes, first.bytes);
}

#[test]
fn the_same_edits_give_the_same_storage() {
    let run = || {
        let mut tree = VoxelTree::new();
        for i in 0..500 {
            let p = pos((i * 31) % 50 - 25, (i * 17) % 40 - 20, (i * 7) % 30);
            tree.set(p, MaterialId((i % 9) as u16)).unwrap();
            if i % 50 == 0 {
                tree.fill_box(cube([i % 30 - 15, 0, 0], 6), MaterialId((i % 4) as u16))
                    .unwrap();
            }
        }
        tree.stats()
    };
    assert_eq!(run(), run());
}

#[test]
fn filling_the_world_is_one_uniform_root() {
    let mut tree = VoxelTree::new();
    assert_eq!(tree.fill_box(VoxelBox::WORLD, STONE), Ok(true));
    assert!(matches!(tree.root(), NodeRef::Uniform(STONE)));
    assert_eq!(tree.stats(), TreeStats::default(), "no node, word or page");
    assert_eq!(
        tree.get(pos(WORLD_MIN_VOXEL, 0, WORLD_END_VOXEL - 1)),
        STONE
    );
    assert_eq!(tree.fill_box(VoxelBox::WORLD, MaterialId::AIR), Ok(true));
    assert!(matches!(tree.root(), NodeRef::Air));
}

#[test]
fn a_box_fill_matches_the_same_voxels_set_one_by_one() {
    let region = VoxelBox::new(pos(-5, -17, 3), pos(13, 2, 40));
    let mut filled = VoxelTree::new();
    filled.set(pos(0, 0, 10), DIRT).unwrap();
    assert_eq!(filled.fill_box(region, STONE), Ok(true));

    let mut by_voxel = VoxelTree::new();
    let (min, end) = (region.min(), region.end());
    for z in (min.z..end.z).rev() {
        for y in min.y..end.y {
            for x in (min.x..end.x).rev() {
                by_voxel.set(pos(x, y, z), STONE).unwrap();
            }
        }
    }
    assert!(same_content(&filled, &by_voxel));
    assert_eq!(live(&filled), live(&by_voxel));
    assert_sound(&filled);
}

#[test]
fn a_covered_brick_becomes_one_uniform_child() {
    let mut tree = VoxelTree::new();
    tree.set(pos(3, 3, 3), DIRT).unwrap();
    tree.fill_box(cube([0, 0, 0], 16), STONE).unwrap();
    // One chain of branches down to level 2, whose child is the brick.
    assert_eq!(live(&tree), (8, 0, 8));
    assert_sound(&tree);
}

#[test]
fn leaf_palettes_grow_and_shrink_through_every_width() {
    let mut tree = VoxelTree::new();
    let leaf_bits = |tree: &VoxelTree| {
        let mut node = tree.root();
        loop {
            match node {
                NodeRef::Branch(b) => node = b.children().next().unwrap().1,
                NodeRef::Leaf(l) => return Some(l.index_bits()),
                _ => return None,
            }
        }
    };
    let slot_pos = |slot: i32| pos(slot % 4, slot / 4 % 4, slot / 16);
    let mut widths = Vec::new();
    for slot in 0..64 {
        tree.set(slot_pos(slot), MaterialId(100 + slot as u16))
            .unwrap();
        widths.push(leaf_bits(&tree));
    }
    for w in [0, 1, 2, 4, 8] {
        assert!(widths.contains(&Some(w)), "width {w} never used");
    }
    for slot in (1..64).rev() {
        tree.set(slot_pos(slot), MaterialId::AIR).unwrap();
        assert_sound(&tree);
    }
    assert_eq!(leaf_bits(&tree), Some(0));
    tree.set(slot_pos(0), MaterialId::AIR).unwrap();
    assert_eq!(live(&tree), (0, 0, 0));
}

#[test]
fn apply_reports_exactly_the_bricks_whose_content_changed() {
    let mut tree = VoxelTree::new();
    tree.set(pos(40, 0, 0), STONE).unwrap();
    let batch = [
        Edit::new(pos(-1, 0, 0), STONE),           // brick (-1, 0, 0)
        Edit::new(pos(0, 0, 0), STONE),            // brick (0, 0, 0)
        Edit::new(pos(15, 15, 15), DIRT),          // brick (0, 0, 0) again
        Edit::new(pos(16, 0, 0), STONE),           // brick (1, 0, 0) ...
        Edit::new(pos(16, 0, 0), MaterialId::AIR), // ... restored
        Edit::new(pos(40, 0, 0), STONE),           // already stone
        Edit::new(pos(0, -16, 0), DIRT),           // brick (0, -1, 0) ...
        Edit::new(pos(0, -16, 0), STONE),          // ... changed twice
    ];
    let dirty = tree.apply(&batch).unwrap();
    assert_eq!(
        dirty.as_slice(),
        [
            BrickPos::new(-1, 0, 0),
            BrickPos::new(0, -1, 0),
            BrickPos::new(0, 0, 0),
        ]
    );
    assert_eq!(tree.get(pos(0, -16, 0)), STONE);
    assert_eq!(tree.get(pos(16, 0, 0)), MaterialId::AIR);
    assert_sound(&tree);
}

// Property test against a dense model.

/// The model covers `[-4, 20)` on every axis: it straddles the world centre,
/// where every level has a node boundary, and holds the whole brick at the
/// origin.
const LO: i32 = -4;
const SIZE: i32 = 24;

fn model_box() -> VoxelBox {
    cube([LO; 3], SIZE as u32)
}

#[derive(Clone, Debug)]
enum Op {
    Set(VoxelPos, MaterialId),
    Fill(VoxelBox, MaterialId),
}

/// Mostly a few materials so regions collapse, sometimes many so palettes
/// reach every width.
fn material() -> impl Strategy<Value = MaterialId> {
    prop_oneof![3 => 0u16..=3, 1 => 0u16..=40].prop_map(MaterialId)
}

fn model_pos() -> impl Strategy<Value = VoxelPos> {
    let axis = || LO..LO + SIZE;
    (axis(), axis(), axis()).prop_map(|(x, y, z)| pos(x, y, z))
}

fn op() -> impl Strategy<Value = Op> {
    let small_box = (model_pos(), [1u32..=8, 1u32..=8, 1u32..=8])
        .prop_map(|(min, size)| VoxelBox::from_min_size(min, size).intersection(&model_box()));
    // Leaf-aligned cubes and the origin brick, which replace whole children.
    let aligned_box = prop_oneof![
        [0..SIZE / 4, 0..SIZE / 4, 0..SIZE / 4].prop_map(|c| cube(c.map(|i| LO + 4 * i), 4)),
        Just(cube([0; 3], 16)),
    ];
    prop_oneof![
        8 => (model_pos(), material()).prop_map(|(p, m)| Op::Set(p, m)),
        1 => (small_box, material()).prop_map(|(b, m)| Op::Fill(b, m)),
        1 => (aligned_box, material()).prop_map(|(b, m)| Op::Fill(b, m)),
    ]
}

/// A plain array over the model box.
struct Dense(Vec<MaterialId>);

impl Dense {
    fn new() -> Self {
        Self(vec![MaterialId::AIR; (SIZE * SIZE * SIZE) as usize])
    }

    fn index(p: VoxelPos) -> usize {
        let [x, y, z] = [p.x - LO, p.y - LO, p.z - LO].map(|v| v as usize);
        x + SIZE as usize * (y + SIZE as usize * z)
    }

    fn get(&self, p: VoxelPos) -> MaterialId {
        if model_box().contains(p) {
            self.0[Self::index(p)]
        } else {
            MaterialId::AIR
        }
    }

    /// Sets every voxel of `region`; returns whether any changed.
    fn fill(&mut self, region: VoxelBox, m: MaterialId) -> bool {
        let mut changed = false;
        for p in positions(region) {
            let cell = &mut self.0[Self::index(p)];
            changed |= *cell != m;
            *cell = m;
        }
        changed
    }
}

/// Every voxel of a box, x fastest.
fn positions(region: VoxelBox) -> impl Iterator<Item = VoxelPos> {
    let (min, end) = (region.min(), region.end());
    (min.z..end.z).flat_map(move |z| {
        (min.y..end.y).flat_map(move |y| (min.x..end.x).map(move |x| pos(x, y, z)))
    })
}

/// Applies `op` to both models. Returns what the tree reported and whether
/// the dense model changed.
fn run(op: &Op, tree: &mut VoxelTree, model: &mut Dense) -> (Result<bool, EditError>, bool) {
    match *op {
        Op::Set(p, m) => (tree.set(p, m), model.fill(cube(p.to_array(), 1), m)),
        Op::Fill(b, m) => (tree.fill_box(b, m), model.fill(b, m)),
    }
}

/// The model box grown by one voxel, to catch writes that leak outside.
fn checked_box() -> VoxelBox {
    cube([LO - 1; 3], SIZE as u32 + 2)
}

proptest! {
    #![proptest_config(world_testkit::profile::proptest_config())]

    #[test]
    fn random_edits_match_the_dense_model_and_stay_canonical(
        ops in vec(op(), 0..=world_testkit::profile::scale().edits_per_case)
    ) {
        let mut tree = VoxelTree::new();
        let mut model = Dense::new();
        for op in &ops {
            let (reported, changed) = run(op, &mut tree, &mut model);
            prop_assert_eq!(reported, Ok(changed), "{:?}", op);
        }
        for p in positions(checked_box()) {
            prop_assert_eq!(tree.get(p), model.get(p), "at {:?}", p);
        }
        assert_sound(&tree);

        // Canonical form: the same content built in other orders has the
        // same structure and the same live storage.
        let mut reversed = VoxelTree::new();
        let all: Vec<VoxelPos> = positions(model_box()).collect();
        for &p in all.iter().rev() {
            reversed.set(p, model.get(p)).unwrap();
        }
        let mut carved = VoxelTree::new();
        carved.fill_box(model_box(), STONE).unwrap();
        for &p in &all {
            carved.set(p, model.get(p)).unwrap();
        }
        for other in [&reversed, &carved] {
            prop_assert!(same_content(&tree, other));
            prop_assert_eq!(live(&tree), live(other));
        }

        prop_assert_eq!(tree.fill_box(model_box(), MaterialId::AIR), Ok(model.fill(model_box(), MaterialId::AIR)));
        prop_assert!(matches!(tree.root(), NodeRef::Air));
        prop_assert_eq!(live(&tree), (0, 0, 0));
    }

    #[test]
    fn apply_reports_the_bricks_that_differ_after_the_batch(
        setup in vec(op(), 0..8),
        batch in vec((model_pos(), material()), 0..=world_testkit::profile::scale().edits_per_case),
    ) {
        let mut tree = VoxelTree::new();
        let mut model = Dense::new();
        for op in &setup {
            run(op, &mut tree, &mut model).0.unwrap();
        }
        let before = model.0.clone();
        let edits: Vec<Edit> = batch.iter().map(|&(p, m)| Edit::new(p, m)).collect();
        for e in &edits {
            model.fill(cube(e.pos.to_array(), 1), e.material);
        }
        let expected = DirtyBricks::from_unsorted(
            positions(model_box())
                .filter(|&p| before[Dense::index(p)] != model.get(p))
                .map(VoxelPos::brick)
                .collect(),
        );
        prop_assert_eq!(tree.apply(&edits), Ok(expected));
        for p in positions(model_box()) {
            prop_assert_eq!(tree.get(p), model.get(p));
        }
        assert_sound(&tree);
    }
}

#[test]
fn snapshot_pages_expose_the_raw_layout_without_copying() {
    let mut tree = VoxelTree::new();
    assert_eq!(tree.snapshot().root_word(), None);
    tree.set(pos(1, 2, 3), STONE).unwrap();
    tree.set(pos(-100, 50, 7), DIRT).unwrap();
    let snap = tree.snapshot();

    let nodes: Vec<(usize, &[Node])> = snap.node_pages().collect();
    let words: Vec<(usize, &[u32])> = snap.word_pages().collect();
    assert_eq!((nodes.len(), words.len()), (1, 1));
    assert_eq!((nodes[0].0, words[0].0), (0, 0));
    assert_eq!(nodes[0].1.len(), NODE_PAGE_LEN);
    assert_eq!(words[0].1.len(), WORD_PAGE_LEN);
    let bytes: &[u8] = bytemuck::cast_slice(nodes[0].1);
    assert_eq!(bytes.len(), NODE_PAGE_LEN * 16);

    // The root word leads to the node a reader sees.
    let root = snap.root_word().unwrap();
    assert_eq!(root & UNIFORM_BIT, 0);
    let node = nodes[0].1[root as usize];
    let NodeRef::Branch(b) = snap.root() else {
        panic!("two voxels need a root branch");
    };
    assert_eq!(node.mask, b.mask());
    assert_eq!(node.info, 0);
    let first = words[0].1[node.base as usize];
    assert_eq!(first & UNIFORM_BIT, 0, "the root's children are nodes");

    tree.fill_box(VoxelBox::WORLD, STONE).unwrap();
    assert_eq!(tree.snapshot().root_word(), Some(UNIFORM_BIT | 1));
}
