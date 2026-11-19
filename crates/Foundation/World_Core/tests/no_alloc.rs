//! Reads never touch the heap: `get`, walking the tree through
//! `TreeRead::root` and `NodeRef`, `same_content`, `content_hash`, and taking,
//! cloning and dropping snapshots allocate zero bytes once the tree exists.
//! The renderer and brick streaming do these every frame (decision 0003), so
//! an allocation here is a per-frame cost and a source of contention.
//!
//! dhat's counting allocator is this test binary's global allocator. It
//! counts every thread, so the file holds a single `#[test]` (libtest then
//! runs nothing in parallel with it) that checks each case in turn and
//! reports every case that allocated.

use std::hint::black_box;

use world_core::serialize::content_hash;
use world_core::tree::{NodeRef, same_content};
use world_core::{MaterialId, TreeRead, VoxelBox, VoxelPos, VoxelTree};

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

/// A tree with every kind of child: a mixed dense block (leaves with 1 to
/// 8-bit indices), uniform boxes, and scattered single voxels.
fn scene() -> VoxelTree {
    let mut tree = VoxelTree::new();
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut next = move |n: u32| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % u64::from(n)) as i32
    };
    for z in 0..24 {
        for y in 0..24 {
            for x in 0..24 {
                let material = MaterialId(1 + next(20) as u16);
                tree.set(VoxelPos::new(x, y, z), material).unwrap();
            }
        }
    }
    tree.fill_box(
        VoxelBox::from_min_size(VoxelPos::new(-256, -64, -256), [256, 64, 256]),
        MaterialId(2),
    )
    .unwrap();
    for _ in 0..2_000 {
        let pos = VoxelPos::new(next(4096) - 2048, next(512) - 256, next(4096) - 2048);
        tree.set(pos, MaterialId(1 + next(4) as u16)).unwrap();
    }
    tree
}

/// Visits every child below `node`, reading everything a reader can read,
/// and returns a checksum of what it saw so nothing is optimized away.
fn walk(node: NodeRef<'_>) -> u64 {
    match node {
        NodeRef::Air => 1,
        NodeRef::Uniform(m) => 2 + u64::from(m.0),
        NodeRef::Branch(b) => {
            let mut sum = u64::from(b.level()) ^ b.mask();
            for (slot, child) in b.children() {
                sum = sum
                    .wrapping_mul(31)
                    .wrapping_add(u64::from(slot) + walk(child));
            }
            sum
        }
        NodeRef::Leaf(l) => {
            let mut sum = l.mask() ^ u64::from(l.index_bits());
            for m in l.palette() {
                sum = sum.wrapping_add(u64::from(m.0));
            }
            for &w in l.index_words() {
                sum = sum.wrapping_add(u64::from(w));
            }
            for slot in 0..64 {
                sum = sum.wrapping_add(u64::from(l.voxel(slot).0));
            }
            sum
        }
    }
}

/// Runs `body` and returns the heap blocks and bytes it allocated.
fn allocations(body: impl FnOnce()) -> (u64, u64) {
    let before = dhat::HeapStats::get();
    body();
    let after = dhat::HeapStats::get();
    (
        after.total_blocks - before.total_blocks,
        after.total_bytes - before.total_bytes,
    )
}

#[test]
fn reads_allocate_nothing() {
    // Setup allocates freely before the profiler starts counting.
    let tree = scene();
    let twin = scene();
    let snapshot = tree.snapshot();
    let probes: Vec<VoxelPos> = (0..20_000)
        .map(|i| VoxelPos::new(i % 300 - 150, (i / 7) % 100 - 60, (i / 13) % 300 - 150))
        .collect();

    let _profiler = dhat::Profiler::builder().testing().build();
    let cases: [(&str, Box<dyn Fn()>); 7] = [
        (
            "get on the live tree",
            Box::new(|| {
                let solid = probes.iter().filter(|&&p| !tree.get(black_box(p)).is_air());
                black_box(solid.count());
            }),
        ),
        (
            "get on a snapshot",
            Box::new(|| {
                let solid = probes
                    .iter()
                    .filter(|&&p| !snapshot.get(black_box(p)).is_air());
                black_box(solid.count());
            }),
        ),
        (
            "walk through TreeRead::root and NodeRef",
            Box::new(|| {
                black_box(walk(black_box(&tree).root()));
                black_box(walk(black_box(&snapshot).root()));
            }),
        ),
        (
            "same_content",
            Box::new(|| {
                assert!(same_content(black_box(&tree), black_box(&twin)));
                assert!(same_content(black_box(&tree), black_box(&snapshot)));
            }),
        ),
        (
            "content_hash",
            Box::new(|| {
                black_box(content_hash(black_box(&tree)));
                black_box(content_hash(black_box(&snapshot)));
            }),
        ),
        (
            "take, clone and drop snapshots",
            Box::new(|| {
                for _ in 0..100 {
                    let a = black_box(tree.snapshot());
                    let b = black_box(a.clone());
                    drop(a);
                    black_box(b.get(VoxelPos::new(1, 2, 3)));
                    drop(b);
                }
            }),
        ),
        (
            "stats",
            Box::new(|| {
                black_box(tree.stats());
            }),
        ),
    ];

    // Boxing the closures above allocated; only what runs inside counts.
    let mut failures = Vec::new();
    for (name, case) in &cases {
        let (blocks, bytes) = allocations(case);
        if blocks != 0 || bytes != 0 {
            failures.push(format!("{name}: {blocks} allocations, {bytes} bytes"));
        }
    }
    assert!(
        failures.is_empty(),
        "reads allocated:\n  {}",
        failures.join("\n  ")
    );
}
