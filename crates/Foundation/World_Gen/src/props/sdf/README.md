# Signed Distance Fields

## Purpose

This module is the vocabulary of shapes for props. A signed distance field returns the distance from any point to a surface, negative inside. Because it is a function and not a grid, the same description can be sampled at any resolution, which is what makes props resolution independent.

## Scope

- Primitives: sphere, box, rounded box, capsule, cylinder, cone, torus, plane, and heightfield-style slabs.
- Combinators: union, intersection and difference, their smooth variants, offset, shell, and repetition within bounds.
- Transforms: translation, scale, and rotation in a deterministic form, with the matching adjustment of distance values.
- Domain deformations: noise displacement and bend or twist, using the noise module.
- The graph type: a tree or DAG of primitives and combinators, with evaluation, conservative bounding boxes, and material tags attached to leaves.
- Evaluation helpers: gradient by finite difference for normals, and interval or bounding queries so voxelization can skip empty space.

## Out of scope

- Converting fields to voxels: voxelize.
- Generating the structure of trees and ruins: grammar.
- Terrain and cave fields for the whole world: continental and regional, which use their own field-based code.
- Rendering of fields on the GPU.

## Relationships

It depends on noise for displacement and on World_Core coordinate types for bounds. Grammar emits graphs from it, voxelize samples them, and regional may reuse its primitives for caves and cliffs.

## Rules

- Evaluation is deterministic: authoritative paths use operations with identical results across platforms and avoid platform math libraries.
- Bounds are conservative: a field is never nonzero outside its reported box.
- Combinators preserve the sign convention, and any that distort distances document the Lipschitz bound.
- Graphs are immutable once built and safe to evaluate from many threads.
- Primitive and combinator behavior is validated by tests against closed-form values.
