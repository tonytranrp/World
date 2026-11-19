# Chunk Orchestration

## Purpose

This module turns "give me chunk (x, y, z) for this seed" into voxels. It owns the order of stages, the randomness handed to them, the handling of data that crosses chunk borders, and the writing of results into World_Core structures. It is the only module that knows the whole pipeline.

## Scope

- The chunk generation entry: seed, coordinate, requested level of detail and parameters in, a populated World_Core chunk out.
- Deriving independent random streams per stage and per coordinate by hashing seed, a stage tag and the coordinate, so no stage's randomness depends on what ran before it.
- Deciding the margin each stage needs around a chunk so features crossing a border are generated identically from either side, such as a tree whose canopy overhangs the neighbor.
- Caching of intermediate data shared by many chunks, such as coarse heightmaps, with deterministic contents regardless of who computes them first.
- Scheduling many chunks in parallel with rayon, and gathering results in a fixed order.
- Reporting of which chunks a given request touched, so higher layers can invalidate correctly.

## Out of scope

- The generation algorithms themselves: continental, regional, props and surface.
- Deciding which chunks the player needs or when to generate them: World_Render and World_Game.
- Applying player edits over the base: World_Core edit overlay.
- Saving and loading.

## Relationships

It depends on all other World_Gen modules and on World_Core for chunk coordinates and tree writes. Nothing inside World_Gen depends on it. Engine and Game code call it through the crate's public entry point.

## Rules

- Generating a chunk alone or as part of a large batch gives identical voxels.
- Results do not depend on thread count, scheduling, or the order chunks are requested.
- Shared caches are pure memoization: removing them changes speed, never output.
- Border margins are computed, not guessed; seam tests guard them.
- Generation never reads or writes edits.
