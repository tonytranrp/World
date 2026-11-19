//! proptest strategies for voxel positions, edits and rays.

use proptest::collection::vec;
use proptest::prelude::*;
use world_core::{Edit, MaterialId, VoxelBox, VoxelPos};

use crate::dense::{Bounds, EditOp};
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

/// A box inside `bounds`. Each axis picks two cut points in `0..=size`, so
/// thin, full-width and empty boxes all occur; shrinking leads to the empty
/// box at the lowest corner.
pub fn voxel_box(bounds: Bounds) -> impl Strategy<Value = VoxelBox> {
    let lo = bounds.min.to_array();
    let cuts = move |axis: usize| {
        let size = bounds.size[axis];
        (0..=size, 0..=size).prop_map(move |(a, b)| {
            let start = lo[axis] as i64;
            (
                (start + a.min(b) as i64) as i32,
                (start + a.max(b) as i64) as i32,
            )
        })
    };
    (cuts(0), cuts(1), cuts(2)).prop_map(|((x0, x1), (y0, y1), (z0, z1))| {
        VoxelBox::new(VoxelPos::new(x0, y0, z0), VoxelPos::new(x1, y1, z1))
    })
}

/// One step inside `bounds`: seven in eight are single voxels, the rest fill
/// or carve (with air) a random box.
pub fn edit_op(bounds: Bounds, max_material: u16) -> impl Strategy<Value = EditOp> {
    prop_oneof![
        7 => edit(bounds, max_material).prop_map(EditOp::Set),
        1 => (voxel_box(bounds), material(max_material))
            .prop_map(|(region, material)| EditOp::Fill { region, material }),
    ]
}

/// Between zero and `max_len` steps inside `bounds`.
pub fn edit_ops(
    bounds: Bounds,
    max_material: u16,
    max_len: usize,
) -> impl Strategy<Value = Vec<EditOp>> {
    vec(edit_op(bounds, max_material), 0..=max_len)
}

/// A position inside `bounds` that hugs its faces: on each axis independently
/// it lies within `margin` voxels of the low face, of the high face, or
/// anywhere, so edges and corners come up often. Panics on a zero margin or
/// an empty axis.
pub fn edge_pos(bounds: Bounds, margin: u32) -> impl Strategy<Value = VoxelPos> {
    assert!(margin > 0, "edge_pos needs a margin of at least one voxel");
    let axis = move |a: usize| {
        let size = bounds.size[a];
        assert!(size > 0, "edge_pos needs bounds with voxels on every axis");
        let (lo, hi) = (
            bounds.min.to_array()[a] as i64,
            bounds.min.to_array()[a] as i64 + size as i64,
        );
        let m = margin.min(size) as i64;
        prop_oneof![lo..lo + m, hi - m..hi, lo..hi].prop_map(|v| v as i32)
    };
    (axis(0), axis(1), axis(2)).prop_map(|(x, y, z)| VoxelPos::new(x, y, z))
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
    fn boxes_stay_inside_the_bounds_and_can_be_empty() {
        let mut runner = TestRunner::new(Config::with_cases(300));
        let empty = std::cell::Cell::new(0);
        runner
            .run(&voxel_box(bounds()), |b| {
                prop_assert!(bounds().as_box().contains_box(&b));
                prop_assert!(b.min().x <= b.end().x && b.min().y <= b.end().y);
                empty.set(empty.get() + usize::from(b.is_empty()));
                Ok(())
            })
            .unwrap();
        assert!(empty.get() > 0, "no empty box in 300 cases");
    }

    #[test]
    fn ops_stay_inside_the_bounds() {
        let mut runner = TestRunner::new(Config::with_cases(200));
        runner
            .run(&edit_ops(bounds(), 3, 12), |ops| {
                prop_assert!(ops.len() <= 12);
                for op in ops {
                    match op {
                        EditOp::Set(e) => prop_assert!(bounds().contains(e.pos)),
                        EditOp::Fill { region, material } => {
                            prop_assert!(bounds().as_box().contains_box(&region));
                            prop_assert!(material.0 <= 3);
                        }
                    }
                }
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn edge_positions_stay_inside_and_reach_every_corner() {
        let wide = Bounds::new(VoxelPos::new(-1_000, 0, 500), [2_000, 3_000, 10]);
        let mut runner = TestRunner::new(Config::with_cases(2_000));
        let corners = std::cell::RefCell::new(std::collections::BTreeSet::new());
        runner
            .run(&edge_pos(wide, 2), |p| {
                prop_assert!(wide.contains(p));
                if (p.x == -1_000 || p.x == 999) && (p.y == 0 || p.y == 2_999) {
                    corners.borrow_mut().insert((p.x, p.y));
                }
                Ok(())
            })
            .unwrap();
        assert_eq!(corners.borrow().len(), 4, "{:?}", corners.borrow());
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
