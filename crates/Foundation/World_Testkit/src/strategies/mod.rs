//! proptest strategies for voxel positions, edits and rays.

use proptest::collection::vec;
use proptest::prelude::*;
use world_core::{Edit, MaterialId, VoxelPos};

use crate::dense::Bounds;
use crate::hits::Ray;

/// A position anywhere inside `bounds`.
pub fn voxel_pos(bounds: Bounds) -> impl Strategy<Value = VoxelPos> {
    let lo = bounds.min.to_array();
    let [sx, sy, sz] = bounds.size.map(|s| s as i32);
    (0..sx, 0..sy, 0..sz).prop_map(move |(x, y, z)| VoxelPos::new(lo[0] + x, lo[1] + y, lo[2] + z))
}

/// A material from air (`0`) up to and including `max`.
pub fn material(max: u16) -> impl Strategy<Value = MaterialId> {
    (0..=max).prop_map(MaterialId)
}

/// One edit inside `bounds`.
pub fn edit(bounds: Bounds, max_material: u16) -> impl Strategy<Value = Edit> {
    (voxel_pos(bounds), material(max_material)).prop_map(|(pos, material)| Edit::new(pos, material))
}

/// Between zero and `max_len` edits inside `bounds`.
pub fn edits(
    bounds: Bounds,
    max_material: u16,
    max_len: usize,
) -> impl Strategy<Value = Vec<Edit>> {
    vec(edit(bounds, max_material), 0..=max_len)
}

/// A ray starting near `bounds` (inside or up to three voxels outside) heading
/// in a random direction. About one component in eight is exactly zero so
/// axis-aligned rays are covered. Origins are continuous, so a ray essentially
/// never runs exactly along a voxel edge, where hit order is ambiguous.
pub fn ray_near(bounds: Bounds) -> impl Strategy<Value = Ray> {
    let lo = bounds.min.to_array().map(|v| v as f32 - 3.0);
    let hi = [
        lo[0] + bounds.size[0] as f32 + 6.0,
        lo[1] + bounds.size[1] as f32 + 6.0,
        lo[2] + bounds.size[2] as f32 + 6.0,
    ];
    let component = || prop_oneof![1 => Just(0.0f32), 7 => -1.0f32..1.0f32];
    (
        (lo[0]..hi[0], lo[1]..hi[1], lo[2]..hi[2]),
        (component(), component(), component()),
    )
        .prop_filter("direction must not be zero", |(_, d)| *d != (0.0, 0.0, 0.0))
        .prop_map(|((ox, oy, oz), (dx, dy, dz))| Ray {
            origin: [ox, oy, oz],
            dir: [dx, dy, dz],
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::test_runner::{Config, TestRunner};

    fn bounds() -> Bounds {
        Bounds::new(VoxelPos::new(-2, 1, 3), [5, 4, 3])
    }

    #[test]
    fn positions_stay_inside_the_bounds() {
        let mut runner = TestRunner::new(Config::with_cases(200));
        runner
            .run(&voxel_pos(bounds()), |p| {
                prop_assert!(bounds().contains(p));
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn edits_respect_length_and_material_limits() {
        let mut runner = TestRunner::new(Config::with_cases(200));
        runner
            .run(&edits(bounds(), 6, 10), |list| {
                prop_assert!(list.len() <= 10);
                for e in list {
                    prop_assert!(e.material.0 <= 6 && bounds().contains(e.pos));
                }
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn rays_have_a_nonzero_direction() {
        let mut runner = TestRunner::new(Config::with_cases(200));
        runner
            .run(&ray_near(bounds()), |r| {
                prop_assert!(r.dir != [0.0, 0.0, 0.0]);
                Ok(())
            })
            .unwrap();
    }
}
