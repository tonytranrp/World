# Hits

## Purpose

This module checks the GPU ray tracer against the CPU reference exactly. It defines the record of a ray hit and the comparator that sends the same rays through two tracers and reports every place their answers differ.

## Scope

- The hit record: whether the ray hit and, if so, the voxel coordinate, material id, entered face and distance.
- The exact comparator: equality of voxel, material, face and distance for every ray, with no epsilon. A hit on one side and a miss on the other is a mismatch.
- Taking the two sides as plain functions or traits, so any CPU reference, GPU readback or stand-in plugs in.
- Readable mismatch reports: ray index, origin and direction, both hits side by side, and the fields that differ, with the number printed capped and the total count always shown.
- A stand-in tracer built on the dense model's ray cast, so the comparator and its reports are tested before the GPU tracer exists.

## Out of scope

- Running the GPU tracer and reading back its buffers: World_Render and World_Testkit_Bevy.
- The CPU reference tracer: World_Cpu_Ref.
- Generating rays and scenes: strategies and the root `tests/` crate.
- Tolerant image comparison: golden.

## Relationships

Builds on dense for the stand-in and for voxel and face types. Used by World_Cpu_Ref's tests and by the CPU-versus-GPU suite in the root `tests/` folder.

## Rules

- Exactness is the contract. A tolerance is never added here. If tracers differ in the last bit, the tie-breaking rule is fixed in the tracers.
- A mismatch report is enough to reproduce: it carries the ray and the scene identity.
- Misses are compared like hits.
- The comparator is itself checked with deliberately corrupted hit lists.
