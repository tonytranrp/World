//! Where the player's intent touches the world. For now: picking with the
//! exact CPU tracer, and a dig that carves a sphere at the picked voxel, which
//! drives edits, residency and saving end to end.

use bevy::prelude::*;
use world_core::consts::VOXELS_PER_METER;
use world_core::{Edit, MaterialId, VoxelPos};
use world_cpu_ref::camera::Ray;
use world_cpu_ref::trace::{TraceOptions, trace};

use crate::player::PlayerCamera;
use crate::residency::World;

/// Picking reach, in metres.
const REACH_M: f32 = 64.0;

/// The dig's radius, in voxels: half a metre.
const DIG_RADIUS: i32 = 32;

pub(crate) struct InteractionPlugin;

impl Plugin for InteractionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (dig_on_click, build_on_click));
    }
}

/// The voxel the camera looks at, within reach.
fn pick(world: &World, transform: &GlobalTransform) -> Option<VoxelPos> {
    let scale = VOXELS_PER_METER as f32;
    let origin = transform.translation() * scale;
    let dir = transform.forward().as_vec3();
    let ray = Ray {
        origin: origin.to_array(),
        dir: dir.to_array(),
    };
    let options = TraceOptions {
        stop_level: 0,
        max_t: REACH_M * scale,
    };
    let hit = trace(&world.0.snapshot(), &ray, &options).ok()??;
    hit.resident.then_some(hit.voxel)
}

/// The air edits of a ball of `radius` voxels around `centre`.
fn ball(centre: VoxelPos, radius: i32) -> Vec<Edit> {
    let mut edits = Vec::new();
    let r2 = radius * radius;
    for z in -radius..=radius {
        for y in -radius..=radius {
            for x in -radius..=radius {
                if x * x + y * y + z * z <= r2 {
                    let p = VoxelPos::new(centre.x + x, centre.y + y, centre.z + z);
                    if p.in_world() {
                        edits.push(Edit::new(p, MaterialId::AIR));
                    }
                }
            }
        }
    }
    edits
}

fn dig_on_click(
    buttons: Option<Res<ButtonInput<MouseButton>>>,
    world: Option<ResMut<World>>,
    camera: Query<&GlobalTransform, With<PlayerCamera>>,
) {
    // Headless apps have no mouse.
    if !buttons.is_some_and(|b| b.just_pressed(MouseButton::Left)) {
        return;
    }
    let (Some(mut world), Ok(transform)) = (world, camera.single()) else {
        return;
    };
    let Some(target) = pick(&world, transform) else {
        return;
    };
    match world.0.apply(&ball(target, DIG_RADIUS)) {
        Ok(bricks) => debug!("dug at {target:?}: {} bricks changed", bricks.len()),
        Err(e) => warn!("dig at {target:?} refused: {e}"),
    }
}

/// Right click places a cube of stone, a metre wide, on the picked voxel:
/// box edits through the same path as the dig.
fn build_on_click(
    buttons: Option<Res<ButtonInput<MouseButton>>>,
    world: Option<ResMut<World>>,
    camera: Query<&GlobalTransform, With<PlayerCamera>>,
) {
    if !buttons.is_some_and(|b| b.just_pressed(MouseButton::Right)) {
        return;
    }
    let (Some(mut world), Ok(transform)) = (world, camera.single()) else {
        return;
    };
    let Some(target) = pick(&world, transform) else {
        return;
    };
    let size = VOXELS_PER_METER;
    let half = (size / 2) as i32;
    let min = VoxelPos::new(target.x - half, target.y, target.z - half);
    let cube = world_core::VoxelBox::from_min_size(min, [size; 3]);
    if let Err(e) = world.0.fill_box(cube, MaterialId(1)) {
        warn!("build at {target:?} refused: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ball_is_every_voxel_within_its_radius() {
        let edits = ball(VoxelPos::new(0, 0, 0), 2);
        // Lattice points in a sphere of radius 2: 33.
        assert_eq!(edits.len(), 33);
        assert!(edits.iter().all(|e| e.material.is_air()));
    }
}
