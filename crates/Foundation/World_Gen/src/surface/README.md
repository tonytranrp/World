# Surface Detail

## Purpose

This module decides what each solid voxel is made of. After the terrain field says where the ground is, this stage assigns materials: topsoil, subsoil, moss, sand, rock strata, ore and everything else that gives the world its painterly color and its gameplay properties. Because the material table drives density, strength and friction, the choices made here also decide how destruction feels.

## Scope

- Layer rules: choose a material by depth below the surface, slope, altitude, biome and noise, with smooth blending and sharp strata where wanted.
- Rock strata and ore veins from seeded noise bands and cell structures.
- Surface cover such as grass, moss, snow and sand driven by slope, exposure and biome.
- Fine surface variation at the scale of single voxels: speckle and grain that make close-up stone and soil read as sculpted detail.
- Wetness and shore rules near water.
- Mapping from the symbolic layers of a biome to material ids through a rule table that comes in as parameters.

## Out of scope

- The shape of terrain: continental and regional.
- Props and their internal materials: props and voxelize.
- Defining materials and their properties: World_Core material module.
- Dynamic changes such as fire, erosion over time or growth: World_Sim.
- Lighting and rendering of the color.

## Relationships

It depends on noise, on the terrain field from regional, on biome data from continental, and on World_Core material ids. The chunk module runs it last. Props use the same material ids but do not call it.

## Rules

- Material choice is a pure function of position, depth, slope, biome, seed and parameters.
- Depth and slope are computed from the same terrain field everywhere, so borders match and chunk order is irrelevant.
- Rule tables are data: adding or tuning a layer never needs a code change.
- Every rule has a defined fallback so no solid voxel can be left without a material.
- Material ids are used as stable identifiers; unknown ids are rejected rather than guessed.
