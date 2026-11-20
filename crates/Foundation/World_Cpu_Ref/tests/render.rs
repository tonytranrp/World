//! Rendering named scenes to images: a golden image of a small fixed scene,
//! per-pixel agreement with the dense DDA, and a generated-terrain chunk
//! written to a PNG for people to look at.
//!
//! The golden image lives in `tests/golden/` beside this file and changes
//! only with `UPDATE_GOLDEN=1` (World_Testkit's golden rule); a missing
//! golden fails. Rendering uses only exact IEEE operations, so the image is
//! compared exactly on every platform.

use std::path::Path;

use world_core::{DenseGrid, MaterialId, MaterialTable, VoxelPos, VoxelSource};
use world_cpu_ref::camera::{Camera, CameraDesc};
use world_cpu_ref::output::shade;
use world_cpu_ref::render;
use world_cpu_ref::trace::{Face, TraceOptions};
use world_gen::{ChunkCoord, Generator, GeneratorVersion, TerrainConfig};
use world_testkit::dense::{Bounds, DenseWorld, VoxelModel};
use world_testkit::golden::{Rgba8Image, Tolerance, assert_image_golden};
use world_testkit::hits::{self, HitRecord, compare_hits};

const STONE: MaterialId = MaterialId(1);
const DIRT: MaterialId = MaterialId(2);
const GRASS: MaterialId = MaterialId(3);
const WOOD: MaterialId = MaterialId(4);
const GLASS: MaterialId = MaterialId(5);
const LAVA: MaterialId = MaterialId(6);

fn materials() -> MaterialTable {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../assets/config/materials.ron"
    );
    ron::from_str(&std::fs::read_to_string(path).expect("materials.ron")).expect("valid table")
}

/// "Steps and sphere": a 64-voxel cube at (-32, -16, -32) with a stone floor,
/// grass-topped dirt steps, a wood pillar, a thin stone wall, a lava pool and
/// a glass ball. Built voxel by voxel from integer rules.
fn steps_scene() -> DenseGrid {
    let origin = VoxelPos::new(-32, -16, -32);
    let mut g = DenseGrid::new(3, origin);
    for z in -32..32 {
        for y in -16..32 {
            for x in -32..32 {
                let m = scene_voxel(x, y, z);
                if !m.is_air() {
                    g.set(VoxelPos::new(x, y, z), m);
                }
            }
        }
    }
    g
}

fn scene_voxel(x: i32, y: i32, z: i32) -> MaterialId {
    let floor = y < -12;
    // Three steps rising toward -z, each 8 deep and 3 high, topped by grass.
    let step_top = if x < -4 && z < 8 {
        -12 + 3 * ((8 - z) / 8).min(3)
    } else {
        -12
    };
    let in_pool = (4..16).contains(&x) && (12..24).contains(&z) && y == -13;
    let pillar = (20..24).contains(&x) && (-24..-20).contains(&z) && y < 12;
    let wall = x == 0 && (-28..0).contains(&z) && y < 0;
    let (dx, dy, dz) = (x - 14, y + 2, z + 6);
    let ball = dx * dx + dy * dy + dz * dz <= 36;
    if in_pool {
        LAVA
    } else if floor {
        STONE
    } else if y < step_top - 1 {
        DIRT
    } else if y < step_top {
        GRASS
    } else if pillar {
        WOOD
    } else if wall {
        STONE
    } else if ball {
        GLASS
    } else {
        MaterialId::AIR
    }
}

fn steps_camera() -> Camera {
    Camera::new(CameraDesc {
        eye: [44.0, 30.0, 52.0],
        forward: [-44.0, -36.0, -52.0],
        up: [0.0, 1.0, 0.0],
        tan_half_fov_y: 0.6,
        width: 160,
        height: 120,
    })
    .expect("valid camera")
}

fn golden(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join(name)
}

#[test]
fn the_steps_scene_matches_its_golden_image() {
    let grid = steps_scene();
    let gbuffer = render(&grid, &steps_camera(), &TraceOptions::default()).unwrap();
    let image = shade(&gbuffer, &materials());
    let hits = gbuffer.hits().iter().flatten().count();
    assert!(hits > 1000 && hits < gbuffer.hits().len(), "{hits} hits");
    let actual = Rgba8Image::new(image.width(), image.height(), image.into_rgba());
    assert_image_golden(&golden("steps_and_sphere.png"), &actual, Tolerance::EXACT);
}

#[test]
fn every_pixel_of_the_steps_scene_matches_the_dda() {
    let grid = steps_scene();
    let camera = steps_camera();
    let gbuffer = render(&grid, &camera, &TraceOptions::default()).unwrap();
    let mut dense = DenseWorld::new(Bounds::new(VoxelSource::origin(&grid), [64; 3]));
    for z in -32..32 {
        for y in -16..32 {
            for x in -32..32 {
                let m = scene_voxel(x, y, z);
                if !m.is_air() {
                    dense.set(VoxelPos::new(x, y, z), m);
                }
            }
        }
    }
    let mut ours = Vec::new();
    let mut theirs = Vec::new();
    for py in 0..camera.height() {
        for px in 0..camera.width() {
            let ray = camera.ray(px, py).unwrap();
            let hit = gbuffer.hit(px, py).map(|h| hits::Hit {
                pos: h.voxel,
                material: h.material,
                face: face(h.face),
                t: h.t,
            });
            ours.push(HitRecord {
                pixel: (px, py),
                hit,
            });
            let dda = dense.cast_ray(&hits::Ray {
                origin: ray.origin,
                dir: ray.dir,
            });
            theirs.push(HitRecord {
                pixel: (px, py),
                hit: dda,
            });
        }
    }
    compare_hits(&theirs, &ours, 0.0).unwrap();
}

fn face(f: Face) -> hits::Face {
    match f {
        Face::NegX => hits::Face::NegX,
        Face::PosX => hits::Face::PosX,
        Face::NegY => hits::Face::NegY,
        Face::PosY => hits::Face::PosY,
        Face::NegZ => hits::Face::NegZ,
        Face::PosZ => hits::Face::PosZ,
        Face::Inside => hits::Face::Inside,
    }
}

/// A generated chunk's footprint at coarse level 1 (one sample every 4
/// voxels, 256 x 256), built into a 256-voxel grid at a quarter of the
/// height scale, rendered and written to a PNG in the test scratch folder.
#[test]
fn a_generated_terrain_chunk_renders_to_a_png() {
    let g = Generator::new(0x5EED, GeneratorVersion::CURRENT, TerrainConfig::default()).unwrap();
    let chunk = ChunkCoord::new(576, 512, 280).unwrap();
    let coarse = g.coarse_columns(chunk, 1).unwrap();
    let n = coarse.samples_per_axis() as i32;
    let heights: Vec<i32> = coarse
        .columns()
        .iter()
        .map(|c| c.height.div_euclid(4))
        .collect();
    let low = heights.iter().copied().min().unwrap();
    let bottom = low - 8;
    let mut grid = DenseGrid::new(4, VoxelPos::new(0, bottom, 0));
    let layers = coarse.layers();
    for j in 0..n {
        for i in 0..n {
            let column = coarse.column(i as u32, j as u32).unwrap();
            let top = heights[(j * n + i) as usize].min(bottom + 255);
            for y in bottom..top {
                // Sample the column's layers at the matching full-scale depth.
                let m = column.material_at(column.height - 1 - 4 * (top - 1 - y), &layers);
                grid.set(VoxelPos::new(i, y, j), m);
            }
        }
    }
    let camera = Camera::new(CameraDesc {
        eye: [-90.0, (low + 330) as f32, -90.0],
        forward: [1.0, -0.75, 1.0],
        up: [0.0, 1.0, 0.0],
        tan_half_fov_y: 0.55,
        width: 192,
        height: 128,
    })
    .unwrap();
    let gbuffer = render(&grid, &camera, &TraceOptions::default()).unwrap();
    let mut seen = std::collections::BTreeSet::new();
    for hit in gbuffer.hits().iter().flatten() {
        seen.insert(hit.material.0);
    }
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join("terrain_chunk_576_280.png");
    shade(&gbuffer, &materials()).write_png(&path).unwrap();
    println!("wrote {}", path.display());
    let mut surfaces = std::collections::BTreeMap::new();
    for c in coarse.columns() {
        *surfaces.entry(c.surface.0).or_insert(0) += 1;
    }
    println!("surfaces {surfaces:?}");
    assert!(seen.contains(&GRASS.0), "terrain surfaces seen: {seen:?}");
}
