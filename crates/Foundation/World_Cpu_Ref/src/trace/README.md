# Trace

## Purpose

This module casts rays through the voxel tree and returns exact results. It is the reference implementation of what the GPU compute traversal is supposed to compute. It favors obvious correctness over speed and is written independently of the production traversal in World_Core.

## Scope

- Ray versus tree traversal, step by step in the same logical order as the GPU algorithm: enter the root bounds, descend through occupied children, step through empty ones using child masks, and report the first hit.
- Level-limited traversal that stops at a requested coarse level, reproducing how distant rays resolve against mip data.
- The hit result: distance, voxel coordinate, material id, face normal and the level reached.
- Primary-ray evaluation for a whole frame, producing the data the output module records.
- Reference lighting queries for the early lighting stage: occlusion rays toward the sun and short ambient-occlusion rays, with documented sample patterns.
- A deliberately naive path that checks every voxel along the ray, used to validate this module's traversal on small scenes.

## Out of scope

- Production queries for gameplay and physics: World_Core ray module.
- Global illumination, radiance caches, temporal filtering and denoising.
- Box-march rendering of dynamic objects and their GPU BVH. If added to the reference, it is a separate concern, not mixed into tree traversal.
- Image writing and comparison: output.

## Relationships

It depends on World_Core for the tree, coordinates and materials, and on the camera module for rays. It hands results to the output module. World_Render's tests compare GPU hits to what this module produces.

## Rules

- Exact tie-breaking at voxel faces, edges and corners is specified and identical to the GPU rule.
- No shortcuts that change results: any acceleration must be proven equivalent by comparing with the naive path.
- No dependence on thread count, platform or random state; sample patterns are fixed tables.
- Traversal reads only; it never mutates the tree or caches across calls in a way that affects answers.
- Correctness bugs found on the GPU are first reproduced here as a small scene and added to tests.
