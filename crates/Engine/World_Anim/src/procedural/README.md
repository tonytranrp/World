# procedural

## Purpose

This folder generates motion from rules instead of authored clips. It covers procedural walk cycles for creatures, spring-driven secondary motion, and the sway and recoil that make the player's body respond to movement and tool use.

## Scope

- Procedural gait: cycles derived from speed, stride length, and ground contact, producing foot targets (for ik) and body bob for characters and creatures with no authored walk clips.
- Spring-driven secondary motion: damped springs on chains such as tails, hair, loose parts, and carried items, solved with stable integration at the fixed step.
- Player sway and recoil: body lean from acceleration, head and arm sway, and recoil impulses when a tool swings or fires.
- Parameter-driven locomotion style, such as a stride and bounce set per creature type, all as data.
- Blending procedural output with AnimationGraph output through weights.
- The animation-mode state: animated, procedural, or ragdoll, and its transitions.

## Out of scope

- Foot placement on terrain and two-bone solving: ik.
- Rigid-body physics: World_Physics and ragdoll.
- Vegetation sway, which is a shader ray-space warp and moves no voxel data: World_Render.
- Creature behavior and decision-making: World_Game.

## Relationships

Depends on rig (bones, hierarchy) and World_Physics (velocity and ground state from the character controller). Depended on by ik, which receives gait-derived targets, and by the crate's schedule. Sibling of ik.

## Rules

- Output is bone Transform adjustments only; voxel data never changes.
- Springs are stable at the fixed timestep for any stiffness the data allows.
- All tunables come from data, not constants in code.
- Procedural motion is deterministic for the same inputs, so tests can assert on it.
