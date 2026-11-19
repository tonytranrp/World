//! Voxel ray traversal (Amanatides-Woo DDA) over the dense model.
//!
//! Every distance is computed directly as `(boundary - origin) / dir`, never
//! accumulated, so the result is bit-identical to a per-voxel slab test. That
//! is what lets hit comparison demand exact equality.

use world_core::VoxelPos;

use super::world::{DenseWorld, VoxelModel};
use crate::hits::{Face, Hit, HitSource, Ray};

impl DenseWorld {
    /// The first solid voxel along `ray`, or `None`.
    pub fn cast_ray(&self, ray: &Ray) -> Option<Hit> {
        let (o, d) = (ray.origin, ray.dir);
        if d == [0.0, 0.0, 0.0] {
            return None;
        }

        // Clip against the world's box so traversal stops at its far side.
        let bounds = self.bounds();
        let lo = bounds.min.to_array().map(|v| v as f32);
        let hi = [
            lo[0] + bounds.size[0] as f32,
            lo[1] + bounds.size[1] as f32,
            lo[2] + bounds.size[2] as f32,
        ];
        let (mut t_enter, mut t_exit) = (f32::NEG_INFINITY, f32::INFINITY);
        for a in 0..3 {
            if d[a] == 0.0 {
                if o[a] < lo[a] || o[a] > hi[a] {
                    return None;
                }
            } else {
                let (t1, t2) = ((lo[a] - o[a]) / d[a], (hi[a] - o[a]) / d[a]);
                t_enter = t_enter.max(t1.min(t2));
                t_exit = t_exit.min(t1.max(t2));
            }
        }
        if t_exit < 0.0 || t_enter > t_exit {
            return None;
        }

        let mut voxel = [
            o[0].floor() as i32,
            o[1].floor() as i32,
            o[2].floor() as i32,
        ];
        let at = |v: [i32; 3]| VoxelPos::new(v[0], v[1], v[2]);

        let start = self.get(at(voxel));
        if !start.is_air() {
            return Some(Hit {
                pos: at(voxel),
                material: start,
                face: Face::Inside,
                t: 0.0,
            });
        }

        let step: [i32; 3] = d.map(|c| {
            if c > 0.0 {
                1
            } else if c < 0.0 {
                -1
            } else {
                0
            }
        });
        let mut boundary = [0i32; 3];
        let mut t_next = [f32::INFINITY; 3];
        for a in 0..3 {
            boundary[a] = if step[a] > 0 { voxel[a] + 1 } else { voxel[a] };
            if step[a] != 0 {
                t_next[a] = (boundary[a] as f32 - o[a]) / d[a];
            }
        }

        loop {
            // Lowest axis wins a tie, which only matters for rays through voxel edges.
            let axis = if t_next[0] <= t_next[1] && t_next[0] <= t_next[2] {
                0
            } else if t_next[1] <= t_next[2] {
                1
            } else {
                2
            };
            let t = t_next[axis];
            if t > t_exit {
                return None;
            }
            voxel[axis] += step[axis];
            boundary[axis] += step[axis];
            t_next[axis] = (boundary[axis] as f32 - o[axis]) / d[axis];

            let material = self.get(at(voxel));
            if !material.is_air() {
                return Some(Hit {
                    pos: at(voxel),
                    material,
                    face: entry_face(axis, step[axis]),
                    t,
                });
            }
        }
    }
}

/// Moving in `+` along an axis enters a voxel through its `-` face.
fn entry_face(axis: usize, step: i32) -> Face {
    match (axis, step > 0) {
        (0, true) => Face::NegX,
        (0, false) => Face::PosX,
        (1, true) => Face::NegY,
        (1, false) => Face::PosY,
        (2, true) => Face::NegZ,
        _ => Face::PosZ,
    }
}

impl HitSource for DenseWorld {
    fn cast(&self, ray: &Ray) -> Option<Hit> {
        self.cast_ray(ray)
    }
}

/// Test oracle for the traversal above: a slab test against every solid voxel.
/// Slow and structurally different, so a bug in one is unlikely to hide in both.
pub fn brute_force_cast(world: &DenseWorld, ray: &Ray) -> Option<Hit> {
    let (o, d) = (ray.origin, ray.dir);
    let mut best: Option<Hit> = None;
    for (pos, material) in world.solids() {
        let lo = pos.to_array().map(|v| v as f32);
        let (mut t_enter, mut t_exit) = (f32::NEG_INFINITY, f32::INFINITY);
        let mut axis = 0;
        let mut miss = false;
        for a in 0..3 {
            if d[a] == 0.0 {
                if o[a] < lo[a] || o[a] > lo[a] + 1.0 {
                    miss = true;
                }
            } else {
                let (t1, t2) = ((lo[a] - o[a]) / d[a], (lo[a] + 1.0 - o[a]) / d[a]);
                let near = t1.min(t2);
                if near > t_enter {
                    t_enter = near;
                    axis = a;
                }
                t_exit = t_exit.min(t1.max(t2));
            }
        }
        if miss || t_exit < 0.0 || t_enter > t_exit {
            continue;
        }
        let (t, face) = if t_enter < 0.0 {
            (0.0, Face::Inside)
        } else {
            (
                t_enter,
                entry_face(axis, if d[axis] > 0.0 { 1 } else { -1 }),
            )
        };
        if best.is_none_or(|b| t < b.t) {
            best = Some(Hit {
                pos,
                material,
                face,
                t,
            });
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dense::{Bounds, DenseWorld};
    use world_core::MaterialId;

    fn world() -> DenseWorld {
        DenseWorld::new(Bounds::new(VoxelPos::new(0, 0, 0), [8, 8, 8]))
    }

    #[test]
    fn straight_ray_hits_the_near_face() {
        let mut w = world();
        w.set(VoxelPos::new(5, 3, 3), MaterialId(2));
        let hit = w
            .cast_ray(&Ray {
                origin: [0.5, 3.5, 3.5],
                dir: [1.0, 0.0, 0.0],
            })
            .unwrap();
        assert_eq!(hit.pos, VoxelPos::new(5, 3, 3));
        assert_eq!(hit.face, Face::NegX);
        assert_eq!(hit.t, 4.5);
    }

    #[test]
    fn ray_from_inside_a_solid_voxel_reports_inside() {
        let mut w = world();
        w.set(VoxelPos::new(2, 2, 2), MaterialId(1));
        let hit = w
            .cast_ray(&Ray {
                origin: [2.5, 2.5, 2.5],
                dir: [0.0, 1.0, 0.0],
            })
            .unwrap();
        assert_eq!((hit.face, hit.t), (Face::Inside, 0.0));
    }

    #[test]
    fn ray_pointing_away_misses() {
        let mut w = world();
        w.set(VoxelPos::new(5, 3, 3), MaterialId(2));
        assert_eq!(
            w.cast_ray(&Ray {
                origin: [0.5, 3.5, 3.5],
                dir: [-1.0, 0.0, 0.0]
            }),
            None
        );
    }

    #[test]
    fn negative_direction_enters_through_the_positive_face() {
        let mut w = world();
        w.set(VoxelPos::new(2, 4, 4), MaterialId(3));
        let hit = w
            .cast_ray(&Ray {
                origin: [7.5, 4.5, 4.5],
                dir: [-1.0, 0.0, 0.0],
            })
            .unwrap();
        assert_eq!((hit.pos, hit.face), (VoxelPos::new(2, 4, 4), Face::PosX));
    }

    #[test]
    fn ray_that_enters_the_box_from_outside_still_hits() {
        let mut w = world();
        w.set(VoxelPos::new(0, 0, 0), MaterialId(1));
        let hit = w
            .cast_ray(&Ray {
                origin: [-4.5, 0.5, 0.5],
                dir: [1.0, 0.0, 0.0],
            })
            .unwrap();
        assert_eq!(
            (hit.pos, hit.face, hit.t),
            (VoxelPos::new(0, 0, 0), Face::NegX, 4.5)
        );
    }

    #[test]
    fn zero_direction_misses() {
        assert_eq!(
            world().cast_ray(&Ray {
                origin: [1.0; 3],
                dir: [0.0; 3]
            }),
            None
        );
    }

    #[test]
    fn brute_force_agrees_on_a_simple_case() {
        let mut w = world();
        w.set(VoxelPos::new(5, 3, 3), MaterialId(2));
        let ray = Ray {
            origin: [0.5, 3.5, 3.5],
            dir: [1.0, 0.0, 0.0],
        };
        assert_eq!(brute_force_cast(&w, &ray), w.cast_ray(&ray));
    }
}
