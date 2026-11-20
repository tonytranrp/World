//! The hierarchical tracer against World_Testkit's dense DDA (and, for
//! continuous rays, its brute-force slab test), exactly: voxel, face,
//! material and `t` bit for bit (`compare_hits` with a zero epsilon).
//!
//! Sources are `DenseGrid`s and the world `VoxelTree`, filled by random edits
//! and boxes (so uniform subtrees occur). Rays: World_Testkit's continuous
//! rays near the scene, lattice rays (integer and half-integer origins with
//! small rational directions, so rays run exactly along faces and through
//! edges and corners, and axis-aligned rays are common), rays from far
//! outside the scene or the world, rays starting inside solids, and long
//! empty rays. Stop levels are checked against a DDA over the scene
//! coarsened by brute force.

use proptest::prelude::*;
use world_core::{DenseGrid, MaterialId, VoxelPos, VoxelSource, VoxelTree};
use world_cpu_ref::camera::Ray;
use world_cpu_ref::trace::{Face, TraceHit, TraceOptions, trace};
use world_testkit::dense::{Bounds, DenseWorld, EditOp, VoxelModel, brute_force_cast};
use world_testkit::hits::{self, HitRecord, compare_hits};
use world_testkit::profile::proptest_config;
use world_testkit::strategies::{edit_ops, ray_near};

fn to_testkit_face(face: Face) -> hits::Face {
    match face {
        Face::NegX => hits::Face::NegX,
        Face::PosX => hits::Face::PosX,
        Face::NegY => hits::Face::NegY,
        Face::PosY => hits::Face::PosY,
        Face::NegZ => hits::Face::NegZ,
        Face::PosZ => hits::Face::PosZ,
        Face::Inside => hits::Face::Inside,
    }
}

fn to_testkit(hit: TraceHit) -> hits::Hit {
    hits::Hit {
        pos: hit.voxel,
        material: hit.material,
        face: to_testkit_face(hit.face),
        t: hit.t,
    }
}

fn to_testkit_ray(ray: &Ray) -> hits::Ray {
    hits::Ray {
        origin: ray.origin,
        dir: ray.dir,
    }
}

/// Traces with the reference and checks the hit level.
fn traced<S: VoxelSource + ?Sized>(source: &S, ray: &Ray, stop_level: u32) -> Option<hits::Hit> {
    let options = TraceOptions {
        stop_level,
        ..TraceOptions::default()
    };
    let hit = trace(source, ray, &options).expect("valid ray");
    if let Some(h) = hit {
        assert_eq!(h.level, stop_level);
    }
    hit.map(to_testkit)
}

/// Exact comparison of one ray, with the ray in the failure message.
fn check(ours: Option<hits::Hit>, dda: Option<hits::Hit>, ray: &Ray) -> Result<(), TestCaseError> {
    let record = |hit| [HitRecord { pixel: (0, 0), hit }];
    compare_hits(&record(dda), &record(ours), 0.0)
        .map_err(|e| TestCaseError::fail(format!("{ray:?}\n{e}")))
}

/// Applies the same operations to a dense model and a source.
fn build<S>(
    bounds: Bounds,
    ops: &[EditOp],
    source: &mut S,
    mut set: impl FnMut(&mut S, VoxelPos, MaterialId),
) -> DenseWorld {
    let mut dense = DenseWorld::new(bounds);
    for &op in ops {
        dense.apply_op(op);
    }
    for (pos, m) in dense.solids() {
        set(source, pos, m);
    }
    dense
}

fn grid_scene(origin: VoxelPos, levels: u32, ops: &[EditOp]) -> (DenseGrid, DenseWorld) {
    let mut grid = DenseGrid::new(levels, origin);
    let bounds = Bounds::new(origin, [grid.size(); 3]);
    let dense = build(bounds, ops, &mut grid, |g, p, m| {
        g.set(p, m);
    });
    (grid, dense)
}

/// A ray with integer or half-integer origin and a direction of small
/// multiples of a half, near `bounds`.
fn lattice_ray(bounds: Bounds) -> impl Strategy<Value = Ray> {
    let lo = bounds.min.to_array();
    let size = bounds.size;
    let coord = move |a: usize| {
        (lo[a] - 3..lo[a] + size[a] as i32 + 3, any::<bool>())
            .prop_map(|(v, half)| v as f32 + if half { 0.5 } else { 0.0 })
    };
    let component = prop::sample::select(vec![-2.0f32, -1.0, -0.5, 0.0, 0.0, 0.5, 1.0, 2.0]);
    (
        (coord(0), coord(1), coord(2)),
        (component.clone(), component.clone(), component),
    )
        .prop_filter("non-zero direction", |(_, d)| *d != (0.0, 0.0, 0.0))
        .prop_map(|((x, y, z), (dx, dy, dz))| Ray::new([x, y, z], [dx, dy, dz]))
}

/// A ray from `distance` voxels away aimed at a random point of `bounds`.
fn far_ray(bounds: Bounds, distance: f32) -> impl Strategy<Value = Ray> {
    let lo = bounds.min.to_array().map(|v| v as f32);
    let size = bounds.size.map(|s| s as f32);
    (
        (0.0f32..1.0, 0.0f32..1.0, 0.0f32..1.0),
        (-1.0f32..1.0, -1.0f32..1.0, -1.0f32..1.0),
    )
        .prop_filter("a direction to start from", |(_, d)| {
            d.0 * d.0 + d.1 * d.1 + d.2 * d.2 > 0.01
        })
        .prop_map(move |((tx, ty, tz), (ox, oy, oz))| {
            let target = [
                lo[0] + tx * size[0],
                lo[1] + ty * size[1],
                lo[2] + tz * size[2],
            ];
            let len = (ox * ox + oy * oy + oz * oz).sqrt();
            let origin = [
                target[0] + ox / len * distance,
                target[1] + oy / len * distance,
                target[2] + oz / len * distance,
            ];
            let dir = [
                target[0] - origin[0],
                target[1] - origin[1],
                target[2] - origin[2],
            ];
            Ray::new(origin, dir)
        })
}

fn any_ray(bounds: Bounds) -> impl Strategy<Value = Ray> {
    prop_oneof![
        3 => ray_near(bounds).prop_map(|r| Ray::new(r.origin, r.dir)),
        3 => lattice_ray(bounds),
        1 => far_ray(bounds, 200.0),
    ]
}

/// Grid origins, including negative ones and odd offsets.
fn origin() -> impl Strategy<Value = VoxelPos> {
    (-40i32..40, -40i32..40, -40i32..40).prop_map(|(x, y, z)| VoxelPos::new(x, y, z))
}

/// Coarsens `dense` to cells of `4^level` voxels aligned to `origin`: a cell
/// with any solid voxel becomes all its most common material (lowest id on a
/// tie). Brute force, independent of the tracer's own counting.
fn coarsen(dense: &DenseWorld, level: u32) -> DenseWorld {
    let bounds = dense.bounds();
    let span = 1i32 << (2 * level);
    let mut out = DenseWorld::new(bounds);
    let [sx, sy, sz] = bounds.size.map(|s| s as i32);
    let o = bounds.min;
    for cz in (0..sz).step_by(span as usize) {
        for cy in (0..sy).step_by(span as usize) {
            for cx in (0..sx).step_by(span as usize) {
                let mut counts = std::collections::BTreeMap::new();
                for z in 0..span {
                    for y in 0..span {
                        for x in 0..span {
                            let m =
                                dense.get(VoxelPos::new(o.x + cx + x, o.y + cy + y, o.z + cz + z));
                            if !m.is_air() {
                                *counts.entry(m).or_insert(0u32) += 1;
                            }
                        }
                    }
                }
                let mut best = (MaterialId::AIR, 0);
                for (m, n) in counts {
                    if n > best.1 {
                        best = (m, n);
                    }
                }
                if best.1 > 0 {
                    for z in 0..span {
                        for y in 0..span {
                            for x in 0..span {
                                out.set(
                                    VoxelPos::new(o.x + cx + x, o.y + cy + y, o.z + cz + z),
                                    best.0,
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

const GRID: [u32; 3] = [16, 16, 16];

proptest! {
    #![proptest_config(proptest_config())]

    #[test]
    fn grids_match_the_dda_exactly(
        (o, ops, rays) in origin().prop_flat_map(|o| {
            let b = Bounds::new(o, GRID);
            (Just(o), edit_ops(b, 4, 24), prop::collection::vec(any_ray(b), 24))
        }),
    ) {
        let (grid, dense) = grid_scene(o, 2, &ops);
        for ray in &rays {
            check(traced(&grid, ray, 0), dense.cast_ray(&to_testkit_ray(ray)), ray)?;
        }
    }

    #[test]
    fn continuous_rays_match_the_brute_force_slab_test(
        (o, ops, rays) in origin().prop_flat_map(|o| {
            let b = Bounds::new(o, GRID);
            (Just(o), edit_ops(b, 3, 16), prop::collection::vec(ray_near(b), 16))
        }),
    ) {
        let (grid, dense) = grid_scene(o, 2, &ops);
        for r in &rays {
            let ray = Ray::new(r.origin, r.dir);
            check(traced(&grid, &ray, 0), brute_force_cast(&dense, r), &ray)?;
        }
    }

    #[test]
    fn stop_levels_match_a_dda_over_the_coarsened_scene(
        (o, ops, rays, level) in origin().prop_flat_map(|o| {
            let b = Bounds::new(o, GRID);
            (Just(o), edit_ops(b, 4, 12), prop::collection::vec(any_ray(b), 16), 1u32..=2)
        }),
    ) {
        let (grid, dense) = grid_scene(o, 2, &ops);
        let coarse = coarsen(&dense, level);
        for ray in &rays {
            check(traced(&grid, ray, level), coarse.cast_ray(&to_testkit_ray(ray)), ray)?;
        }
    }

    #[test]
    fn the_world_tree_matches_the_dda_exactly(
        (o, ops, rays) in (-524_288i32..524_256, -524_288i32..524_256, -524_288i32..524_256)
            .prop_map(|(x, y, z)| VoxelPos::new(x, y, z))
            .prop_flat_map(|o| {
                let b = Bounds::new(o, [32, 32, 32]);
                (Just(o), edit_ops(b, 4, 16), prop::collection::vec(any_ray(b), 12))
            }),
    ) {
        let bounds = Bounds::new(o, [32, 32, 32]);
        let mut tree = VoxelTree::new();
        let dense = build(bounds, &ops, &mut tree, |t, p, m| {
            t.set(p, m).expect("inside the world");
        });
        let snapshot = tree.snapshot();
        for ray in &rays {
            let dda = dense.cast_ray(&to_testkit_ray(ray));
            check(traced(&tree, ray, 0), dda, ray)?;
            check(traced(&snapshot, ray, 0), dda, ray)?;
        }
    }
}

#[test]
fn rays_from_outside_the_world_enter_through_its_faces() {
    // A small scene against the world's +X face.
    let mut tree = VoxelTree::new();
    let bounds = Bounds::new(VoxelPos::new(524_280, -4, -4), [8, 8, 8]);
    let mut dense = DenseWorld::new(bounds);
    for (i, p) in [
        VoxelPos::new(524_287, 0, 0),
        VoxelPos::new(524_283, 2, -3),
        VoxelPos::new(524_281, -4, 3),
    ]
    .into_iter()
    .enumerate()
    {
        let m = MaterialId(i as u16 + 1);
        tree.set(p, m).unwrap();
        dense.set(p, m);
    }
    let rays = [
        Ray::new([524_300.5, 0.5, 0.5], [-1.0, 0.0, 0.0]),
        Ray::new([600_000.0, 0.25, 0.75], [-1.0, 0.0, 0.0]),
        Ray::new([524_310.0, 12.0, -13.0], [-1.0, -0.5, 0.5]),
        Ray::new([524_288.0, 0.5, 0.5], [-1.0, 0.0, 0.0]),
        Ray::new([524_288.0, 0.5, 0.5], [1.0, 0.0, 0.0]),
        Ray::new([524_281.5, 40.0, 3.5], [0.0, -1.0, 0.0]),
    ];
    for ray in &rays {
        let dda = dense.cast_ray(&to_testkit_ray(ray));
        check(traced(&tree, ray, 0), dda, ray).unwrap();
    }
    assert!(traced(&tree, &rays[0], 0).is_some());
    assert_eq!(
        traced(&tree, &rays[4], 0),
        None,
        "pointing out of the world"
    );
}

#[test]
fn long_empty_rays_miss_quickly_and_inside_rays_hit_at_zero() {
    let mut tree = VoxelTree::new();
    tree.set(VoxelPos::new(3, 3, 3), MaterialId(2)).unwrap();
    // Across the whole empty world along a diagonal.
    let miss = Ray::new([-524_000.5, -523_990.25, -524_010.75], [1.0, 1.0, 1.0]);
    assert_eq!(trace(&tree, &miss, &TraceOptions::default()).unwrap(), None);
    let inside = Ray::new([3.5, 3.5, 3.5], [0.0, -1.0, 0.0]);
    let hit = trace(&tree, &inside, &TraceOptions::default())
        .unwrap()
        .unwrap();
    assert_eq!(
        (hit.face, hit.t, hit.voxel),
        (Face::Inside, 0.0, VoxelPos::new(3, 3, 3))
    );
}

#[test]
fn max_distance_turns_far_hits_into_misses() {
    let mut grid = DenseGrid::new(2, VoxelPos::ORIGIN);
    grid.set(VoxelPos::new(10, 1, 1), MaterialId(1));
    let ray = Ray::new([0.5, 1.5, 1.5], [1.0, 0.0, 0.0]);
    let at = |max_t| {
        trace(
            &grid,
            &ray,
            &TraceOptions {
                stop_level: 0,
                max_t,
            },
        )
        .unwrap()
    };
    assert_eq!(at(9.5).map(|h| h.t), Some(9.5));
    assert_eq!(at(9.4), None);
}

#[test]
fn a_whole_cube_stop_level_is_one_cell() {
    let mut grid = DenseGrid::new(2, VoxelPos::ORIGIN);
    grid.set(VoxelPos::new(10, 1, 1), MaterialId(4));
    let ray = Ray::new([-5.5, 1.5, 1.5], [1.0, 0.0, 0.0]);
    let hit = trace(
        &grid,
        &ray,
        &TraceOptions {
            stop_level: 2,
            max_t: f32::INFINITY,
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        (hit.voxel, hit.level, hit.material, hit.t),
        (VoxelPos::new(0, 1, 1), 2, MaterialId(4), 5.5)
    );
    assert_eq!(hit.cell(grid.origin()).size(), [16, 16, 16]);
}

#[test]
fn bad_inputs_are_errors_not_panics() {
    use world_cpu_ref::camera::RayError;
    use world_cpu_ref::trace::TraceError;
    let grid = DenseGrid::new(1, VoxelPos::ORIGIN);
    let ok = Ray::new([0.0; 3], [1.0, 0.0, 0.0]);
    let opts = TraceOptions::default();
    assert_eq!(
        trace(&grid, &Ray::new([0.0; 3], [0.0; 3]), &opts),
        Err(TraceError::Ray(RayError::ZeroDirection))
    );
    assert_eq!(
        trace(
            &grid,
            &Ray::new([f32::NAN, 0.0, 0.0], [1.0, 0.0, 0.0]),
            &opts
        ),
        Err(TraceError::Ray(RayError::NonFinite))
    );
    assert_eq!(
        trace(
            &grid,
            &ok,
            &TraceOptions {
                stop_level: 2,
                max_t: 1.0
            }
        ),
        Err(TraceError::BadStopLevel {
            stop_level: 2,
            levels: 1
        })
    );
    assert_eq!(
        trace(
            &grid,
            &ok,
            &TraceOptions {
                stop_level: 0,
                max_t: f32::NAN
            }
        ),
        Err(TraceError::BadMaxT)
    );
    // Huge but finite coordinates neither panic nor loop.
    let far = Ray::new([3.0e38, -3.0e38, 1.0], [-1.0, 1.0e-30, 0.0]);
    assert_eq!(trace(&grid, &far, &opts), Ok(None));
}
