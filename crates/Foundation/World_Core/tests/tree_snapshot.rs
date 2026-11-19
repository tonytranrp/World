//! Snapshots: each keeps reading exactly the content it was taken from, no
//! matter what the writer does later, how many other snapshots exist, which
//! of them were dropped, or which thread reads it.

use proptest::collection::vec;
use proptest::prelude::*;
use world_core::serialize::encode;
use world_core::tree::NodeRef;
use world_core::{Edit, MaterialId, TreeRead, TreeSnapshot, VoxelBox, VoxelPos, VoxelTree};
use world_testkit::dense::{
    Bounds, DenseWorld, EditOp, VoxelModel, assert_tree_agrees, check_canonical,
};
use world_testkit::profile::{proptest_config, scale};
use world_testkit::strategies::edit_ops;

/// Crosses the origin, so edits split and merge nodes of every level.
const B: Bounds = Bounds::new(VoxelPos::new(-20, -9, -14), [40, 24, 30]);

/// The live tree and the dense model, edited in step.
struct Pair {
    tree: VoxelTree,
    dense: DenseWorld,
}

impl Pair {
    fn new() -> Self {
        Self {
            tree: VoxelTree::new(),
            dense: DenseWorld::new(B),
        }
    }

    fn apply(&mut self, ops: &[EditOp]) {
        for &op in ops {
            self.tree.apply_op(op);
            self.dense.apply_op(op);
        }
    }

    /// A snapshot and the dense copy it must keep matching.
    fn snapshot(&self) -> (TreeSnapshot, DenseWorld) {
        (self.tree.snapshot(), self.dense.clone())
    }
}

proptest! {
    #![proptest_config(proptest_config())]

    #[test]
    fn a_snapshot_never_sees_later_edits(
        before in edit_ops(B, 4, scale().edits_per_case),
        after in edit_ops(B, 4, scale().edits_per_case),
    ) {
        let mut pair = Pair::new();
        pair.apply(&before);
        let (snap, frozen) = pair.snapshot();
        let bytes = encode(&snap);
        pair.apply(&after);

        prop_assert_eq!(assert_tree_agrees(&snap, &frozen, B), Ok(()));
        prop_assert_eq!(encode(&snap), bytes);
        prop_assert!(check_canonical(&snap).is_ok());
        prop_assert_eq!(assert_tree_agrees(&pair.tree, &pair.dense, B), Ok(()));
    }

    #[test]
    fn many_snapshots_each_keep_their_moment(
        rounds in vec(edit_ops(B, 4, scale().edits_per_case / 4), 1..8),
        after in edit_ops(B, 4, scale().edits_per_case),
    ) {
        let mut pair = Pair::new();
        let mut kept = Vec::new();
        for ops in &rounds {
            pair.apply(ops);
            kept.push(pair.snapshot());
        }
        pair.apply(&after);
        for (i, (snap, frozen)) in kept.iter().enumerate() {
            prop_assert_eq!(assert_tree_agrees(snap, frozen, B), Ok(()), "snapshot {}", i);
        }
        prop_assert_eq!(assert_tree_agrees(&pair.tree, &pair.dense, B), Ok(()));
    }

    /// Dropping snapshots frees the pages only they held; the writer and the
    /// survivors must not notice.
    #[test]
    fn dropping_snapshots_then_editing_stays_correct(
        rounds in vec(edit_ops(B, 4, scale().edits_per_case / 4), 1..8),
        dropped in any::<u8>(),
        after in edit_ops(B, 4, scale().edits_per_case),
    ) {
        let mut pair = Pair::new();
        let mut kept = Vec::new();
        for ops in &rounds {
            pair.apply(ops);
            kept.push(pair.snapshot());
        }
        let mut index = 0;
        kept.retain(|_| {
            index += 1;
            dropped >> (index - 1) & 1 == 0
        });
        pair.apply(&after);
        for (snap, frozen) in &kept {
            prop_assert_eq!(assert_tree_agrees(snap, frozen, B), Ok(()));
        }
        prop_assert_eq!(assert_tree_agrees(&pair.tree, &pair.dense, B), Ok(()));
        prop_assert!(check_canonical(&pair.tree).is_ok());
    }
}

/// A small deterministic generator for the threaded run.
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn pos(&mut self) -> VoxelPos {
        let mut axis = |a: usize| B.min.to_array()[a] + (self.next() % u64::from(B.size[a])) as i32;
        VoxelPos::new(axis(0), axis(1), axis(2))
    }

    /// One in sixteen steps fills a box up to 8 voxels wide; the rest set one voxel.
    fn op(&mut self) -> EditOp {
        let material = MaterialId((self.next() % 5) as u16);
        if self.next().is_multiple_of(16) {
            let size = [0; 3].map(|_| 1 + (self.next() % 8) as u32);
            let region = VoxelBox::from_min_size(self.pos(), size).intersection(&B.as_box());
            EditOp::Fill { region, material }
        } else {
            EditOp::Set(Edit::new(self.pos(), material))
        }
    }
}

#[test]
fn snapshots_are_send_sync_and_clone() {
    fn shareable<T: Send + Sync + Clone + 'static>() {}
    shareable::<TreeSnapshot>();
}

#[test]
fn a_snapshot_reads_on_other_threads_while_the_writer_edits() {
    let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
    let mut pair = Pair::new();
    let setup: Vec<EditOp> = (0..2_000).map(|_| rng.op()).collect();
    pair.apply(&setup);
    let (snap, frozen) = pair.snapshot();

    std::thread::scope(|s| {
        // One reader borrows the snapshot, the other owns a clone of it.
        let (owned, frozen) = (snap.clone(), &frozen);
        let readers = [
            s.spawn(|| {
                for _ in 0..3 {
                    assert_eq!(assert_tree_agrees(&snap, frozen, B), Ok(()));
                }
            }),
            s.spawn(move || {
                for _ in 0..3 {
                    assert_eq!(assert_tree_agrees(&owned, frozen, B), Ok(()));
                }
            }),
        ];
        let edits: Vec<EditOp> = (0..2_000).map(|_| rng.op()).collect();
        pair.apply(&edits);
        for reader in readers {
            reader.join().unwrap();
        }
    });

    assert_eq!(assert_tree_agrees(&pair.tree, &pair.dense, B), Ok(()));
    assert_eq!(assert_tree_agrees(&snap, &frozen, B), Ok(()));
}

#[test]
fn a_snapshot_of_a_new_tree_is_air() {
    let tree = VoxelTree::new();
    let snap = tree.snapshot();
    assert!(matches!(snap.root(), NodeRef::Air));
}

#[test]
fn a_snapshot_equals_its_clones_and_not_the_edited_tree() {
    let mut tree = VoxelTree::new();
    tree.fill_box(B.as_box(), MaterialId(2)).unwrap();
    let snap = tree.snapshot();
    assert!(snap == snap.clone());
    assert!(snap == tree.snapshot());
    tree.set(VoxelPos::ORIGIN, MaterialId(3)).unwrap();
    assert!(snap != tree.snapshot());
    assert_eq!(snap.get(VoxelPos::ORIGIN), MaterialId(2));
    assert_eq!(TreeRead::get(&tree, VoxelPos::ORIGIN), MaterialId(3));
}
