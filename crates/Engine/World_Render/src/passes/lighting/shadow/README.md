# Shadow

## Purpose

Sun visibility. For every shaded pixel, a shadow ray is traced toward the sun through the voxel tree (and dynamic-object boxes) to decide whether direct sunlight reaches the surface. This is the first and cheapest lighting stage, and it is what makes terrain, props and the player's own body cast shadows.

## Scope

- Dispatch of the shadow-ray compute pass at the internal resolution, reading world position reconstructed from G-buffer depth.
- Sun direction and disc size, including soft shadow sampling and a bias strategy suited to 1/64 m voxels.
- Occlusion by dynamic objects, including the player body, which must appear in shadows even though the head is hidden in the main first-person view.
- A shadow mask or visibility target consumed by the final lighting combine, with temporal filtering where it reduces noise.
- Distance handling: far shadow rays may use coarse mips from the pool.

## Out of scope

- Ambient occlusion: `ao`. Bounce and indirect light: `gi`.
- Traversal implementation: shared WESL modules, imported here.
- Shadows for rasterized triangle content: Bevy's own passes, or the tier-A extras in `lighting`.
- Sun and sky parameters: owned by the sibling lighting configuration.

## Relationships

Depends on `gbuffer`, on `brick_cache` pool access and on `dynamic_box` data. It runs after primary visibility and feeds the combine stage in `lighting`, then `upscale`.

## Rules

- Shadow rays use the same traversal as primary rays; a shadow decision must never disagree with visible geometry.
- Rays cap their step count and distance; exceeding it counts as lit.
- Missing pool data is treated as non-occluding for this frame and requested through feedback.
- Quality settings reduce ray count rather than switch algorithms.
