# anchors

## Purpose

The terrain is one enormous connected solid, so an island found in it is only free to fall if nothing holds it up. This folder decides what holds things up. It defines anchors, usually bedrock, and decides whether an island of terrain has a connection path to one.

## Scope

- The definition of an anchor: bedrock below a world depth, voxels flagged as unbreakable or fixed, and any region gameplay marks as fixed.
- Path-to-anchor tests: for each island produced by island detection, deciding whether any voxel path connects it to an anchor, using searches bounded to the affected area rather than the whole world.
- Handling terrain edge cases: islands touching loaded-but-incomplete chunks, and regions where the generated base is not yet available, treated conservatively as anchored.
- Classification output: each island is marked supported (stays in the world) or unsupported (released to fall as a new body).
- Re-evaluation: when a supporting piece is removed, neighbors that depended on it are rechecked.
- Time-sliced operation under the same per-frame voxel budget.

## Out of scope

- Finding the islands: islands.
- Producing falling bodies: debris.
- Load-based collapse where an anchored structure fails under weight: World_Sim/stress.
- Marking voxels as anchors in saved data: World_Core.

## Relationships

Depends on islands (its input), World_Core (tree queries), and the destruction budget. Depended on by debris, which acts on the unsupported set, and by World_Sim/stress, which extends anchored-or-not into graded support.

## Rules

- Unsure means supported: the system never drops terrain on incomplete data.
- Searches are bounded and resumable; a pending result is distinct from an answer.
- Anchor definitions come from voxel and material data, not from magic positions in code.
- The same world state always yields the same classification.
