# Regional Scale

## Purpose

This module carves and shapes the medium-scale features of the terrain: caves, rivers, cliffs, overhangs and ravines. They are all expressed as signed distance fields, which can be combined with the macro heightfield to make a solid-or-empty decision at any point in space. This is the level where the landscape stops being a smooth heightmap and becomes a place.

## Scope

- Cave systems from three-dimensional noise thresholds and worm tunnels built from seeded random walks that produce chains of capsules.
- Rivers and lakes: water-carrying channels following the macro heightfield, with width, depth and bank profiles.
- Cliffs, terraces, overhangs and ravines created by shaping fields and domain warping.
- A unified terrain field: a function that, given a position, combines the macro height with the regional fields to say whether it is solid and what its distance to the surface is.
- Feature indexing: deterministic discovery of which features intersect a chunk and its margin, without scanning the whole world.
- Water and other volumes as separate fields so later stages and simulation can treat them distinctly.

## Out of scope

- The macro height, continents and biomes: continental.
- Individual props: props.
- Material layers on the surface: surface.
- Hydrology simulation and runtime fluids: World_Sim.
- The noise primitives: noise.

## Relationships

It depends on noise, on continental sampling interfaces, and on the sdf vocabulary from props where shapes are shared. Surface depends on its terrain field to know depth and slope. Chunk requests its features.

## Rules

- Everything is a pure function of seed, parameters and position; random walks consume seeded streams in a fixed order.
- Features crossing chunk borders are found from either side with identical geometry.
- Cost is bounded per chunk, and feature indexing prevents work proportional to world size.
- Carving never produces floating or open results that depend on chunk order.
