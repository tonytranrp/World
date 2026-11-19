//! Ray hits and the exact comparison between a CPU reference and a GPU tracer.
//!
//! Positions are in voxel units: the voxel at `(x, y, z)` occupies
//! `[x, x + 1) x [y, y + 1) x [z, z + 1)`.

use std::fmt;

use world_core::{MaterialId, VoxelPos};

/// The face of a voxel a ray entered through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    NegX,
    PosX,
    NegY,
    PosY,
    NegZ,
    PosZ,
    /// The ray started inside the voxel.
    Inside,
}

/// The first solid voxel a ray touched.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub pos: VoxelPos,
    pub material: MaterialId,
    pub face: Face,
    /// Distance along the ray, in units of the ray direction's length.
    pub t: f32,
}

/// A ray in voxel units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    pub origin: [f32; 3],
    pub dir: [f32; 3],
}

/// Anything that can answer "what does this ray hit first?": the CPU reference
/// renderer, the dense model, or a read-back of the GPU tracer.
pub trait HitSource {
    fn cast(&self, ray: &Ray) -> Option<Hit>;
}

/// The result for one pixel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitRecord {
    pub pixel: (u32, u32),
    pub hit: Option<Hit>,
}

/// A pinhole camera, enough to generate the same rays for both tracers.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub eye: [f32; 3],
    pub forward: [f32; 3],
    pub up: [f32; 3],
    pub vertical_fov_radians: f32,
}

fn sub_scaled(a: [f32; 3], b: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] + b[0] * s, a[1] + b[1] * s, a[2] + b[2] * s]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / len, v[1] / len, v[2] / len]
}

impl Camera {
    /// One ray per pixel through the pixel centre, row-major, top row first.
    pub fn rays(&self, width: u32, height: u32) -> Vec<((u32, u32), Ray)> {
        let forward = normalize(self.forward);
        let right = normalize(cross(forward, self.up));
        let up = cross(right, forward);
        let half_h = (self.vertical_fov_radians * 0.5).tan();
        let half_w = half_h * width as f32 / height as f32;

        let mut rays = Vec::with_capacity((width * height) as usize);
        for y in 0..height {
            for x in 0..width {
                let u = ((x as f32 + 0.5) / width as f32 * 2.0 - 1.0) * half_w;
                let v = (1.0 - (y as f32 + 0.5) / height as f32 * 2.0) * half_h;
                let dir = sub_scaled(sub_scaled(forward, right, u), up, v);
                rays.push((
                    (x, y),
                    Ray {
                        origin: self.eye,
                        dir: normalize(dir),
                    },
                ));
            }
        }
        rays
    }
}

/// Casts the given rays through a source.
///
/// To compare two tracers, generate the rays once and pass the same slice to
/// both. Generating them twice is not safe: floating-point functions such as
/// `tan` may differ between runs under tools like Miri, and between machines.
pub fn trace_rays(source: &impl HitSource, rays: &[((u32, u32), Ray)]) -> Vec<HitRecord> {
    rays.iter()
        .map(|&(pixel, ray)| HitRecord {
            pixel,
            hit: source.cast(&ray),
        })
        .collect()
}

/// Casts every camera ray through a source.
pub fn trace(source: &impl HitSource, camera: &Camera, width: u32, height: u32) -> Vec<HitRecord> {
    trace_rays(source, &camera.rays(width, height))
}

/// Why two hit lists differ.
#[derive(Clone, Debug, PartialEq)]
pub enum HitMismatch {
    LengthDiffers {
        cpu: usize,
        gpu: usize,
    },
    PixelDiffers {
        index: usize,
        pixel: (u32, u32),
        cpu: Option<Hit>,
        gpu: Option<Hit>,
    },
}

impl fmt::Display for HitMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LengthDiffers { cpu, gpu } => {
                write!(f, "CPU produced {cpu} hit records, GPU produced {gpu}")
            }
            Self::PixelDiffers {
                index,
                pixel,
                cpu,
                gpu,
            } => write!(
                f,
                "hit differs at record {index}, pixel ({}, {}):\n  cpu: {cpu:?}\n  gpu: {gpu:?}",
                pixel.0, pixel.1
            ),
        }
    }
}

fn same_hit(a: &Option<Hit>, b: &Option<Hit>, t_epsilon: f32) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.pos == b.pos
                && a.material == b.material
                && a.face == b.face
                && (a.t == b.t || (a.t - b.t).abs() <= t_epsilon)
        }
        _ => false,
    }
}

/// Compares two hit lists. Voxel, material and face must match exactly. The
/// distance must match within `t_epsilon`; use `0.0` for bit-for-bit equality.
/// Returns the first difference.
pub fn compare_hits(
    cpu: &[HitRecord],
    gpu: &[HitRecord],
    t_epsilon: f32,
) -> Result<(), HitMismatch> {
    if cpu.len() != gpu.len() {
        return Err(HitMismatch::LengthDiffers {
            cpu: cpu.len(),
            gpu: gpu.len(),
        });
    }
    for (index, (c, g)) in cpu.iter().zip(gpu).enumerate() {
        if c.pixel != g.pixel || !same_hit(&c.hit, &g.hit, t_epsilon) {
            return Err(HitMismatch::PixelDiffers {
                index,
                pixel: c.pixel,
                cpu: c.hit,
                gpu: g.hit,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(x: i32, material: u16, t: f32) -> Hit {
        Hit {
            pos: VoxelPos::new(x, 0, 0),
            material: MaterialId(material),
            face: Face::NegX,
            t,
        }
    }

    fn record(pixel: (u32, u32), hit: Option<Hit>) -> HitRecord {
        HitRecord { pixel, hit }
    }

    #[test]
    fn identical_lists_match() {
        let a = [record((0, 0), Some(hit(1, 2, 3.0))), record((1, 0), None)];
        assert_eq!(compare_hits(&a, &a, 0.0), Ok(()));
    }

    #[test]
    fn wrong_material_is_reported_with_its_pixel() {
        let cpu = [record((0, 0), None), record((1, 0), Some(hit(1, 2, 3.0)))];
        let gpu = [record((0, 0), None), record((1, 0), Some(hit(1, 9, 3.0)))];
        match compare_hits(&cpu, &gpu, 0.0).unwrap_err() {
            HitMismatch::PixelDiffers { index, pixel, .. } => {
                assert_eq!((index, pixel), (1, (1, 0)))
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn miss_versus_hit_differs() {
        let cpu = [record((0, 0), None)];
        let gpu = [record((0, 0), Some(hit(0, 1, 1.0)))];
        assert!(compare_hits(&cpu, &gpu, 0.0).is_err());
    }

    #[test]
    fn distance_epsilon_is_honoured() {
        let cpu = [record((0, 0), Some(hit(1, 2, 3.0)))];
        let gpu = [record((0, 0), Some(hit(1, 2, 3.0000002)))];
        assert!(compare_hits(&cpu, &gpu, 0.0).is_err());
        assert!(compare_hits(&cpu, &gpu, 1e-5).is_ok());
    }

    #[test]
    fn length_difference_is_its_own_error() {
        let a = [record((0, 0), None)];
        assert_eq!(
            compare_hits(&a, &[], 0.0),
            Err(HitMismatch::LengthDiffers { cpu: 1, gpu: 0 })
        );
    }

    #[test]
    fn mismatch_message_names_both_sides() {
        let text = HitMismatch::PixelDiffers {
            index: 3,
            pixel: (4, 5),
            cpu: None,
            gpu: Some(hit(0, 1, 1.0)),
        }
        .to_string();
        assert!(text.contains("(4, 5)") && text.contains("cpu") && text.contains("gpu"));
    }

    #[test]
    fn camera_generates_one_unit_ray_per_pixel() {
        let camera = Camera {
            eye: [0.0, 0.0, 0.0],
            forward: [0.0, 0.0, 1.0],
            up: [0.0, 1.0, 0.0],
            vertical_fov_radians: 1.0,
        };
        let rays = camera.rays(4, 3);
        assert_eq!(rays.len(), 12);
        for (_, ray) in rays {
            let len = ray.dir.iter().map(|c| c * c).sum::<f32>().sqrt();
            assert!((len - 1.0).abs() < 1e-5);
            assert!(ray.dir[2] > 0.0);
        }
    }
}
