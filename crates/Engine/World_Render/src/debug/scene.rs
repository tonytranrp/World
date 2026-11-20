//! A small deterministic voxel scene for the window and for tests.
//!
//! It sits in front of a camera at the origin looking down -Z (Bevy's
//! default): a stone floor 1.5 m below the eye, pillars, a dirt sphere, a
//! wooden frame and a glass pane, from 2 to 10 m away. All boxes are written
//! with `fill_box`; the sphere voxel by voxel.

use world_core::{BrickPos, DirtyBricks, MaterialId, VoxelBox, VoxelPos, VoxelTree};

pub const STONE: MaterialId = MaterialId(1);
pub const DIRT: MaterialId = MaterialId(2);
pub const GRASS: MaterialId = MaterialId(3);
pub const WOOD: MaterialId = MaterialId(4);
pub const GLASS: MaterialId = MaterialId(5);
pub const LAVA: MaterialId = MaterialId(6);

fn fill(tree: &mut VoxelTree, min: [i32; 3], end: [i32; 3], m: MaterialId) {
    let region = VoxelBox::new(VoxelPos::from(min), VoxelPos::from(end));
    // Every box here is inside the world, so the edit cannot fail.
    let _ = tree.fill_box(region, m);
}

/// Builds the scene.
pub fn build() -> VoxelTree {
    let mut tree = VoxelTree::new();
    // Floor: 8 m by 8 m of stone, 12 voxels thick, grass on top.
    fill(&mut tree, [-256, -108, -704], [256, -96, -128], STONE);
    fill(&mut tree, [-256, -96, -704], [256, -94, -128], GRASS);
    // Pillars.
    for (i, x) in [-192, -64, 64, 192].into_iter().enumerate() {
        let m = if i % 2 == 0 { STONE } else { WOOD };
        fill(&mut tree, [x - 10, -94, -520], [x + 10, 40, -500], m);
    }
    // A wooden frame with a glass pane.
    fill(&mut tree, [-80, -94, -300], [80, -84, -292], WOOD);
    fill(&mut tree, [-80, 10, -300], [80, 20, -292], WOOD);
    fill(&mut tree, [-80, -84, -300], [-70, 10, -292], WOOD);
    fill(&mut tree, [70, -84, -300], [80, 10, -292], WOOD);
    fill(&mut tree, [-70, -84, -297], [70, 10, -295], GLASS);
    // A dirt sphere with a lava core, half a metre across.
    let (c, r) = ([110, -40, -400], 32);
    for z in -r..=r {
        for y in -r..=r {
            for x in -r..=r {
                let d2 = x * x + y * y + z * z;
                if d2 <= r * r {
                    let m = if d2 <= 8 * 8 { LAVA } else { DIRT };
                    let _ = tree.set(VoxelPos::new(c[0] + x, c[1] + y, c[2] + z), m);
                }
            }
        }
    }
    tree
}

/// The bricks overlapping `region`.
fn bricks_of(region: VoxelBox, out: &mut Vec<BrickPos>) {
    let lo = BrickPos::containing(region.min());
    let end = region.end();
    let hi = BrickPos::containing(VoxelPos::new(end.x - 1, end.y - 1, end.z - 1));
    for z in lo.z..=hi.z {
        for y in lo.y..=hi.y {
            for x in lo.x..=hi.x {
                out.push(BrickPos::new(x, y, z));
            }
        }
    }
}

/// The edit of frame `frame` of the animated scene: every fourth frame a
/// carved hole moves along the floor, the previous one filled back in.
/// Returns the bricks that may have changed (a superset), empty on frames
/// without an edit.
pub fn animate(tree: &mut VoxelTree, frame: u32) -> DirtyBricks {
    if !frame.is_multiple_of(4) {
        return DirtyBricks::default();
    }
    let step = (frame / 4) % 48;
    let x = -192 + step as i32 * 8;
    let strip_stone = VoxelBox::new(
        VoxelPos::new(-256, -108, -260),
        VoxelPos::new(256, -96, -236),
    );
    let strip_grass = VoxelBox::new(
        VoxelPos::new(-256, -96, -260),
        VoxelPos::new(256, -94, -236),
    );
    let carve = VoxelBox::new(
        VoxelPos::new(x, -100, -260),
        VoxelPos::new(x + 24, -94, -236),
    );
    let mut changed = Vec::new();
    for (region, m) in [
        (strip_stone, STONE),
        (strip_grass, GRASS),
        (carve, MaterialId::AIR),
    ] {
        // Inside the world, so the edit cannot fail.
        let _ = tree.fill_box(region, m);
        bricks_of(region, &mut changed);
    }
    DirtyBricks::from_unsorted(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scene_is_in_front_of_the_default_camera() {
        let tree = build();
        assert_eq!(tree.get(VoxelPos::new(0, -100, -400)), STONE);
        assert_eq!(tree.get(VoxelPos::new(0, -95, -400)), GRASS);
        assert_eq!(tree.get(VoxelPos::new(110, -40, -400)), LAVA);
        assert_eq!(tree.get(VoxelPos::new(0, 0, 0)), MaterialId::AIR);
    }

    #[test]
    fn animation_changes_bricks_and_is_deterministic() {
        let mut a = build();
        let mut b = build();
        let da = animate(&mut a, 40);
        let db = animate(&mut b, 40);
        assert!(!da.is_empty());
        assert_eq!(da, db);
        assert!(a == b);
    }
}
