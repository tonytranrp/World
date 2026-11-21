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
            NodeRef::Air | NodeRef::NotResident => {}
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
                if let Some((header, len)) = b.brick_layout() {
                    // A header, one word for every other full leaf and a
                    // record for every leaf but the full ones of the default
                    // material: the most common full material.
                    let mut counts = std::collections::BTreeMap::new();
                    for (_, c) in &children {
                        if let NodeRef::Uniform(m) = c {
                            *counts.entry(*m).or_insert(0usize) += 1;
                        }
                    }
                    let best = counts
                        .iter()
                        .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
                        .map(|(&m, &n)| (m, n));
                    match best {
                        Some((m, n)) => {
                            assert_eq!(header.default, m, "the most common full leaf");
                            assert_eq!(header.full.count_ones() as usize, n);
                        }
                        None => assert_eq!(header, Default::default()),
                    }
                    let full = children
                        .iter()
                        .filter(|(_, c)| matches!(c, NodeRef::Uniform(_)))
                        .fold(0u64, |m, (slot, _)| m | 1 << slot);
                    assert_eq!(header.full | header.other, full, "every full leaf");
                    assert_eq!(header.full & header.other, 0);
                    let others = header.other.count_ones() as usize;
                    let records = children.len() - full.count_ones() as usize;
                    let header_len = if others == 0 { 3 } else { 5 };
                    assert_eq!(len, header_len + others + RECORD_WORDS * records);
                    found.2 += len;
                } else {
                    found.2 += children.len();
                }
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
                // One material lives in the record; more need a palette block.
                if palette.len() > 1 {
                    found.2 += 1 + palette_words(palette.len()) + indices;
                }
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
    assert!(tree.shares_everything_with(&before));
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
    assert!(tree.shares_everything_with(&before));
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
    // Every chunk went with its region, pages and all; only the top store's
    // pages remain, its nodes back on the free list.
    assert!(grown.chunks > 0);
    assert_eq!(cleared.chunks, 0);
    assert!(
        cleared.bytes < grown.bytes,
        "dropping chunks returns their pages"
    );
    assert!(cleared.free_nodes <= grown.branches + grown.free_nodes);
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

/// Bytes of live storage: 16 per node and 4 per word.
fn live_bytes(tree: &VoxelTree) -> usize {
    let s = tree.stats();
    s.branches * size_of::<Node>() + s.words * size_of::<u32>()
}

#[test]
fn a_single_material_leaf_is_one_three_word_record() {
    let mut tree = VoxelTree::new();
    tree.set(pos(0, 0, 0), STONE).unwrap();
    let one = live_bytes(&tree);
    // A second leaf in the same brick adds only its record.
    tree.set(pos(4, 0, 0), STONE).unwrap();
    tree.set(pos(5, 1, 0), STONE).unwrap();
    assert_eq!(live_bytes(&tree) - one, 12);
    // Level 2 and up: eight child words; the brick: a header and two records.
    assert_eq!(live(&tree), (9, 2, 8 + 3 + 2 * RECORD_WORDS));

    let NodeRef::Leaf(l) = brick_child(&tree, pos(4, 0, 0)) else {
        panic!("a partly solid leaf");
    };
    assert_eq!(l.palette().collect::<Vec<_>>(), [STONE]);
    assert_eq!((l.index_bits(), l.index_words()), (0, &[][..]));
    assert_eq!(l.voxel(path::child_slot([1, 1, 0], 0)), STONE);
    assert_eq!(l.voxel(1), MaterialId::AIR);
    assert_sound(&tree);
}

#[test]
fn a_two_material_leaf_adds_a_palette_block() {
    let mut tree = VoxelTree::new();
    tree.set(pos(0, 0, 0), STONE).unwrap();
    let one = live_bytes(&tree);
    // 32 solid voxels of two materials: a record plus a header, one palette
    // word and one word of 1-bit indices.
    for i in 0..32 {
        let m = if i % 3 == 0 { DIRT } else { STONE };
        tree.set(pos(4 + i % 4, i / 4 % 4, i / 16), m).unwrap();
    }
    assert_eq!(live_bytes(&tree) - one, 4 * (RECORD_WORDS + 3));
    let NodeRef::Leaf(l) = brick_child(&tree, pos(4, 0, 0)) else {
        panic!("a mixed leaf");
    };
    assert_eq!(l.palette().collect::<Vec<_>>(), [STONE, DIRT]);
    assert_eq!((l.index_bits(), l.index_words().len()), (1, 1));
    assert_sound(&tree);

    // Down to one material, the palette block is freed again.
    for i in (0..32).step_by(3) {
        tree.set(pos(4 + i % 4, i / 4 % 4, i / 16), STONE).unwrap();
    }
    assert_eq!(live_bytes(&tree) - one, 4 * RECORD_WORDS);
    assert_sound(&tree);
}

/// The leaf-sized child of the tree that holds `p`.
fn brick_child(tree: &VoxelTree, p: VoxelPos) -> NodeRef<'_> {
    let local = path::world_local(p).unwrap();
    let mut node = tree.root();
    for level in (1..TREE_DEPTH).rev() {
        let NodeRef::Branch(b) = node else {
            return node;
        };
        node = b.child(path::child_slot(local, level));
    }
    node
}

#[test]
fn apply_lets_the_last_edit_of_a_voxel_win_in_any_order() {
    let a = pos(1, 1, 1);
    let b = pos(-300, 70, 2);
    let batch = [
        Edit::new(b, DIRT),
        Edit::new(a, STONE),
        Edit::new(b, MaterialId::AIR),
        Edit::new(a, DIRT),
        Edit::new(b, STONE),
        Edit::new(a, MaterialId(3)),
    ];
    let mut tree = VoxelTree::new();
    let dirty = tree.apply(&batch).unwrap();
    assert_eq!((tree.get(a), tree.get(b)), (MaterialId(3), STONE));
    assert_eq!(dirty.as_slice(), {
        let mut bricks = [a.brick(), b.brick()];
        bricks.sort();
        bricks
    });
    let mut sequential = VoxelTree::new();
    for e in batch {
        sequential.set(e.pos, e.material).unwrap();
    }
    assert!(same_content(&tree, &sequential));
    assert_eq!(live(&tree), live(&sequential));

    // Restoring the original value as the last edit reports nothing.
    let restore = [Edit::new(a, DIRT), Edit::new(a, MaterialId(3))];
    assert!(tree.apply(&restore).unwrap().is_empty());
    assert_sound(&tree);
}

#[test]
fn apply_rebuilds_each_node_once_and_matches_single_sets() {
    // A whole brick in one batch: every leaf of one level-1 node.
    let edits: Vec<Edit> = (0..16 * 16 * 16)
        .rev()
        .map(|i| {
            let p = pos(i % 16, i / 16 % 16, i / 256);
            Edit::new(p, MaterialId(1 + (i % 5) as u16))
        })
        .collect();
    let mut batch = VoxelTree::new();
    assert_eq!(
        batch.apply(&edits).unwrap().as_slice(),
        [BrickPos::new(0, 0, 0)]
    );
    let mut single = VoxelTree::new();
    for e in &edits {
        single.set(e.pos, e.material).unwrap();
    }
    assert!(same_content(&batch, &single));
    assert_eq!(live(&batch), live(&single));
    assert_eq!(batch.stats().free_words, 0, "nothing was built twice");
    assert_eq!(batch.stats().free_nodes, 0);
    assert_sound(&batch);
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

    /// A tiny region around the world centre, so batches repeat voxels often
    /// and cross node boundaries of every level.
    #[test]
    fn apply_with_many_repeats_matches_the_edits_in_order(
        setup in vec(op(), 0..4),
        batch in vec(([-2i32..2, -2i32..2, -2i32..2], material()), 0..200),
    ) {
        let mut tree = VoxelTree::new();
        let mut model = Dense::new();
        for op in &setup {
            run(op, &mut tree, &mut model).0.unwrap();
        }
        let edits: Vec<Edit> = batch.iter().map(|&(p, m)| Edit::new(p.into(), m)).collect();
        tree.apply(&edits).unwrap();
        for e in &edits {
            model.fill(cube(e.pos.to_array(), 1), e.material);
        }
        for p in positions(checked_box()) {
            prop_assert_eq!(tree.get(p), model.get(p), "at {:?}", p);
        }
        assert_sound(&tree);
    }

    /// The cursor, the direct lookup and the generic view descent agree with
    /// the model on scattered points, small steps and jumps out of the world.
    #[test]
    fn every_lookup_path_matches_the_model(
        ops in vec(op(), 0..=world_testkit::profile::scale().edits_per_case),
        probes in vec(probe(), 1..400),
    ) {
        let mut tree = VoxelTree::new();
        let mut model = Dense::new();
        for op in &ops {
            run(op, &mut tree, &mut model).0.unwrap();
        }
        let snapshot = tree.snapshot();
        let mut cursor = tree.cursor();
        let mut snap_cursor = snapshot.cursor();
        let mut at = pos(0, 0, 0);
        for probe in probes {
            at = match probe {
                Probe::Jump(p) => p,
                Probe::Step(d) => pos(at.x + d[0], at.y + d[1], at.z + d[2]),
            };
            let expected = model.get(at);
            prop_assert_eq!(cursor.get(at), expected, "cursor at {:?}", at);
            prop_assert_eq!(snap_cursor.get(at), expected, "snapshot cursor at {:?}", at);
            prop_assert_eq!(tree.get(at), expected, "get at {:?}", at);
            let generic = read::descend(tree.root(), TREE_DEPTH, WORLD_ORIGIN, at);
            prop_assert_eq!(generic, expected, "descent at {:?}", at);
        }
    }
}

/// One move of a lookup sequence.
#[derive(Clone, Debug)]
enum Probe {
    /// Anywhere: inside the model, far away, or outside the world.
    Jump(VoxelPos),
    /// A small step from the last position, often within one leaf.
    Step([i32; 3]),
}

fn probe() -> impl Strategy<Value = Probe> {
    let far = || {
        prop_oneof![
            WORLD_MIN_VOXEL - 2..WORLD_MIN_VOXEL + 2,
            -100_000..100_000,
            WORLD_END_VOXEL - 2..WORLD_END_VOXEL + 2,
        ]
    };
    prop_oneof![
        3 => model_pos().prop_map(Probe::Jump),
        1 => (far(), far(), far()).prop_map(|(x, y, z)| Probe::Jump(pos(x, y, z))),
        6 => [-2i32..=2, -2i32..=2, -2i32..=2].prop_map(Probe::Step),
    ]
}

#[test]
fn a_cursor_scan_matches_get_everywhere_and_follows_its_snapshot() {
    let mut tree = VoxelTree::new();
    for i in 0..3000 {
        let p = pos((i * 7) % 40 - 20, (i * 13) % 36 - 18, (i * 5) % 44 - 22);
        tree.set(p, MaterialId(1 + (i % 6) as u16)).unwrap();
    }
    tree.fill_box(cube([-20, -20, -20], 12), DIRT).unwrap();
    let region = cube([-24, -24, -24], 48);
    let snapshot = tree.snapshot();
    let mut cursor = snapshot.cursor();
    let expected: Vec<(VoxelPos, MaterialId)> =
        positions(region).map(|p| (p, tree.get(p))).collect();
    // Forward, x fastest, then backward: every kind of shared prefix.
    for &(p, m) in expected.iter().chain(expected.iter().rev()) {
        assert_eq!(cursor.get(p), m, "at {p:?}");
    }

    // The tree moves on; the snapshot's cursor keeps the old content.
    tree.fill_box(region, STONE).unwrap();
    for &(p, m) in expected.iter().step_by(7) {
        assert_eq!(cursor.get(p), m, "at {p:?}");
        assert_eq!(tree.cursor().get(p), STONE);
    }
}

#[test]
fn cursors_on_air_and_uniform_roots_need_no_nodes() {
    let mut tree = VoxelTree::new();
    assert_eq!(tree.cursor().get(pos(1, 2, 3)), MaterialId::AIR);
    tree.fill_box(VoxelBox::WORLD, DIRT).unwrap();
    let mut cursor = tree.cursor();
    assert_eq!(cursor.get(pos(WORLD_MIN_VOXEL, 0, 0)), DIRT);
    assert_eq!(cursor.get(pos(WORLD_END_VOXEL, 0, 0)), MaterialId::AIR);
    assert_eq!(cursor.get(pos(5, 5, 5)), DIRT);
}

#[test]
fn views_are_canonical_voxel_sources() {
    let mut tree = VoxelTree::new();
    tree.set(pos(0, 0, 0), STONE).unwrap();
    tree.set(pos(1, 0, 0), DIRT).unwrap();
    tree.fill_box(cube([4, 0, 0], 4), STONE).unwrap();
    assert_eq!(VoxelSource::levels(&tree), TREE_DEPTH);
    assert_eq!(VoxelSource::origin(&tree), VoxelBox::WORLD.min());
    assert_eq!(VoxelSource::get(&tree, pos(1, 0, 0)), DIRT);

    let NodeRef::Leaf(_) = brick_child(&tree, pos(0, 0, 0)) else {
        panic!("a mixed leaf");
    };
    let leaf = brick_child(&tree, pos(0, 0, 0));
    assert_eq!(leaf.kind(), NodeKind::Leaf);
    assert_eq!(leaf.mask(), 0b11);
    assert_eq!(leaf.palette().as_slice(), [STONE, DIRT]);
    let uniform = brick_child(&tree, pos(4, 0, 0));
    assert_eq!(uniform.kind(), NodeKind::Uniform(STONE));
    assert_eq!(uniform.mask(), u64::MAX);
    assert_eq!(uniform.palette().as_slice(), [STONE]);
    assert_eq!(uniform.child(9).voxel(3), STONE);
    let air = brick_child(&tree, pos(8, 0, 0));
    assert_eq!((air.kind(), air.mask()), (NodeKind::Air, 0));
    assert_eq!(air.palette().as_slice(), []);
    assert_eq!(
        Palette::from_voxels([DIRT, MaterialId::AIR, STONE, DIRT]).as_slice(),
        [STONE, DIRT]
    );
}

/// The child of `tree` that stands at `level` (where a level-`level` node
/// would) and holds `p`.
fn child_at_level(tree: &VoxelTree, p: VoxelPos, level: u32) -> NodeRef<'_> {
    let local = path::world_local(p).unwrap();
    let mut node = tree.root();
    for parent in (level + 1..TREE_DEPTH).rev() {
        node = node.child(path::child_slot(local, parent));
    }
    node
}

proptest! {
    #![proptest_config(world_testkit::profile::proptest_config())]

    /// A grid and the node-aligned region of the world tree with the same
    /// voxels have identical canonical views: same kinds, masks and voxels
    /// all the way down.
    #[test]
    fn a_grid_reads_like_the_matching_subtree(
        levels in 1u32..=3,
        corner in [-2i32..2, -2i32..2, -2i32..2],
        writes in vec(([0u32..64, 0u32..64, 0u32..64], material()), 0..300),
        filled in material(),
    ) {
        let size = 1u32 << (2 * levels);
        let origin = VoxelPos::from(corner.map(|c| c * size as i32));
        let mut grid = DenseGrid::new(levels, origin);
        let mut tree = VoxelTree::new();
        // Start from a filled half sometimes, so uniform regions split.
        let half = VoxelBox::from_min_size(origin, [size, size, size / 2]);
        tree.fill_box(half, filled).unwrap();
        for p in positions(half) {
            grid.set(p, filled);
        }
        for &(offset, m) in &writes {
            let [x, y, z] = offset.map(|v| (v % size) as i32);
            let p = pos(origin.x + x, origin.y + y, origin.z + z);
            prop_assert_eq!(grid.set(p, m), tree.set(p, m).unwrap());
        }
        let subtree = child_at_level(&tree, origin, levels - 1);
        prop_assert!(read::same_node(grid.root(), subtree));
        prop_assert!(read::same_node(subtree, grid.root()));
        for p in positions(grid.bounds()).step_by(7) {
            prop_assert_eq!(grid.get(p), tree.get(p));
        }
        // Equal content at different places or sizes is not the same source.
        prop_assert!(!same_content(&grid, &tree));
        prop_assert!(same_content(&grid, &grid.clone()));
    }
}
