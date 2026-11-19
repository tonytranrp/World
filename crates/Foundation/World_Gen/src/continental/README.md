# Continental Scale

## Purpose

This module generates the macro shape of the world: where land and sea are, how high the terrain rises, and which biome covers each region. It works on a coarse grid across the whole fixed world, which is large but bounded, and everything finer is built on top of what it decides.

## Scope

- Continent masks and a base heightmap from layered noise (fractal Brownian motion with domain warping).
- Biome assignment from fields such as temperature, moisture and altitude, producing biome identifiers and blend weights.
- An optional hydraulic erosion pass over a coarse heightmap, run with a fixed iteration order so results are reproducible.
- Sampling interfaces: query height, slope and biome at any world position by deterministic interpolation of the coarse fields.
- Coarse data tiling so that any chunk can request just the tiles it needs and recompute them identically.

## Out of scope

- Caves, rivers, cliffs and other local carving: regional.
- Individual rocks, trees and ruins: props.
- Soil, moss and strata layers: surface.
- The noise primitives themselves: noise.
- Deciding chunk order or border margins: chunk.

## Relationships

It depends on noise and on World_Core coordinate types. Regional, props and surface read its sampling interfaces. The chunk module requests its coarse tiles.

## Rules

- Everything is a pure function of seed, parameters and position.
- Erosion and any iterative smoothing must use a fixed traversal order and fixed iteration counts, with no dependence on thread scheduling. If it is parallelized, the combination of partial results must be order independent.
- Bounded world: the fields cover the fixed map and define explicit behavior at its edge.
- Tile boundaries are invisible: the value at a position never depends on which tile computed it.
- Parameters come in as data; no tunable values are hardcoded in algorithms.
