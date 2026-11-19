# ik

## Purpose

This folder adjusts animated poses to fit the world. Its central job is making feet stand on uneven voxel ground, where fine 1/64 m terrain makes any fixed walk clip land feet in the air or in the floor. It also covers aiming and look-at.

## Scope

- Two-bone IK: an analytic solver for limbs with a hinge middle joint (thigh and shin, upper and lower arm), given a target position and a pole direction, respecting the rig's joint limits.
- Foot placement: sampling the ground below each foot through World_Physics queries, finding height and surface normal at the 1/16 m collision resolution, and moving the foot target there.
- Pelvis adjustment: lowering or raising the hips so the lower of the two feet reaches the ground without overextending a leg.
- Foot locking and smoothing so planted feet do not slide and corrections do not pop.
- Look-at and aim constraints for head, eyes, and held tools, distributing rotation along the spine within limits.
- Hand targets for gripping tools and props.
- Blend weights, so IK can fade in and out according to state (airborne, ragdoll, or being driven by a clip).

## Out of scope

- Procedural gait, springs, and secondary motion: procedural.
- The skeleton data itself: rig.
- Physics simulation of limbs: ragdoll.
- Terrain collision queries themselves: World_Physics.

## Relationships

Depends on rig (bones, limits) and World_Physics (ground queries). Depended on by the crate's schedule, which runs it after clip sampling. Sibling of procedural, which may supply targets and blend weights to it.

## Rules

- IK modifies bone transforms only; it never touches voxel data.
- Solvers are pure functions of poses and targets, so they are unit-testable without a Bevy App.
- Every result respects the rig's joint limits.
- Ground queries are bounded per frame.
