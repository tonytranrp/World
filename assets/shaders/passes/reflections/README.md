# Reflections Pass Shaders

## Purpose

This folder holds the shaders for reflections: mirror-like water, glossy floors and shiny props. Reflection rays are traced through the same voxel structure as primary rays, so the voxel world reflects at low extra cost. The player's body must also appear in reflections, so reflection rays additionally test dynamic-object boxes.

## Scope

- Reflection ray generation from G-buffer position, normal and roughness; roughness controls sharpness by spreading rays.
- Tracing through the world tree using `../../traversal`.
- Intersection against dynamic objects through a small GPU BVH over their oriented boxes, then box-march into each hit object.
- Shading at the reflection hit point, using the lighting results available to it, with fog applied consistently with the primary view.
- Reduced-resolution operation, restricted to smooth materials, for lower-end hardware tiers.
- Denoising or filtering of the reflection result.

## Out of scope

- Primary visibility (`../primary`) and dynamic-object drawing (`../dynamic_box`).
- Diffuse lighting and GI (`../lighting`).
- Building and refitting the dynamic-object BVH. That is Rust (and compute dispatch) in `World_Render`; this folder only traverses it.
- The decision of how reflections should look (mirror-sharp, glossy or painterly blur) is a tuning parameter in `assets/config`, not hard-coded here.

## Relationships

- Reads the G-buffer; imports `../../traversal` and `../../common`.
- Mirrors the reflections pass module in `World_Render/src/passes`.
- Output is combined during lighting or composite according to the frame order.

## Rules

- Reflection rays reuse shared traversal; no separate tree-walking code.
- The full player body, including the head hidden from the first-person view, is visible in reflections.
- Resolution and material eligibility are driven by bindings and config so the low-end tier can run at half resolution.
- Runs on compute-only baseline hardware.
