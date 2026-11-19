# Voxelization

## Purpose

This module converts a distance field graph into voxels at a requested resolution. It is where resolution independence is realized: the same prop description becomes fine voxels up close and coarser voxels at a lower level of detail, with matching shape and consistent materials.

## Scope

- Sampling a graph over a region at a given voxel size, using bounds to skip empty space and a hierarchical refinement that only subdivides near the surface.
- The occupancy rule: how the sign and distance at a voxel center determine solid or empty, and how thin features are preserved at coarse levels.
- Material assignment: resolving the symbolic material tags of the graph into material ids, including layered or gradient rules inside a prop.
- Producing output in World_Core form, such as bricks or tree fragments, aligned to the global voxel grid and chunk boundaries.
- Clipping to the requesting chunk and its margin, so a prop crossing chunks yields identical voxels from every chunk.
- Level-of-detail consistency with the mip rule in World_Core, so a coarse voxelization agrees with downsampling a fine one as closely as practical.

## Out of scope

- Defining shapes and trees: sdf and grammar.
- Where and when a prop appears: props and chunk.
- Writing into the world tree beyond handing over voxels: World_Core mutation.
- Terrain voxelization from heightfields: continental and surface paths.

## Relationships

It depends on sdf for evaluation, on World_Core for voxel and chunk coordinates, materials and tree types, and on noise only indirectly. The props module and the chunk module call it.

## Rules

- Output is a pure function of graph, resolution and region; evaluation order and thread count never matter.
- Sample positions are defined in integer voxel coordinates, so the voxel grid is global and borders align.
- Rules at coarse levels are deterministic and documented, with no random dithering unless seeded from the coordinate.
- Memory is bounded per call; large props are processed in pieces.
