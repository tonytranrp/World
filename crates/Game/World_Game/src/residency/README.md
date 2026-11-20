# Residency

## Purpose

The residency domain decides which parts of the 16 km world are in memory, in which form, and keeps every player edit forever. It turns decision 0006's tiers into a running system: full detail near the player, coarse detail beyond, compressed edited chunks in a warm cache, and everything the player changed in region files and an edit log on disk.

## Scope

- Planning: which chunks must be loaded at full detail and which coarse chunks are wanted, from the player's position, with hysteresis so chunks near the edge do not thrash.
- Loading: generating chunks from the seed, merging saved edit overlays, and decoding warm or saved chunks, on worker threads, then joining them to the world tree.
- Evicting: dropping chunks that are far away, saving the edited ones first, within the RAM budgets.
- Persistence orchestration: the write-ahead edit log, region file writes on a single I/O thread, checkpoints and crash recovery.
- The game's edit entry point for the world: edits to loaded chunks apply at once, edits to unloaded chunks wait and apply when the chunk loads.
- The Bevy glue: the configs, the world resource and the per-frame update driven by the player camera.

## Out of scope

- The tree, overlays, region and log formats: World_Core.
- Terrain generation: World_Gen.
- Uploading to the GPU and the brick cache: World_Render.
- Save slots, menus and player state: the `save` domain.

## Relationships

Depends on World_Core and World_Gen. Feeds World_Render with the frame's snapshot and changed bricks. `interaction` and `tools` send edits through it; `save` asks it to flush.

## Rules

- The CPU tree stays the single source of truth; workers build standalone chunks and only the main thread changes the world.
- No edit is ever dropped: it is logged before the frame ends and applied when its chunk is loaded.
- Everything that decides output is ordered (`BTreeMap`, sorted lists), so the same inputs give the same world.
- Budgets and radii are data in `assets/config/residency.ron`.
