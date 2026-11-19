# Tree Mip Levels

## Purpose

This module derives coarse representations of the tree for distant rays. Rays beyond the near field stop at higher levels of the tree, and the GPU's ray-guided brick cache requests coarse data for them, so GPU memory scales with screen resolution rather than world size. This module defines what a coarse voxel means and keeps it correct as the world changes.

## Scope

- The downsampling rule that turns a 4x4x4 group of children into one parent-level representative: occupancy, dominant or highest-priority material, and the tie-break order.
- Storage of per-node coarse summaries, such as representative material and occupancy fraction, sufficient for far shading and for conservative skipping.
- Incremental refresh: after an edit, only the ancestors along the edited paths are recomputed, bottom-up.
- Level queries: reading the coarse value of any region at a requested level.
- Rebuild-from-scratch used for verification and after bulk construction.

## Out of scope

- Choosing which level a pixel needs: that depends on screen-space size and belongs to World_Render and the ray module's level parameter.
- GPU texture mips and image filtering.
- Generator-side LOD decisions: World_Gen props voxelize handles resolution for procedural content.
- Raw node layout and allocation: node and alloc.

## Relationships

It reads leaves and nodes through the node and alloc modules and runs after the mutate module finishes a write. The ray module consumes it for level-limited traversal; World_Render streams it; World_Cpu_Ref uses the same rule so the reference matches the GPU.

## Rules

- The downsampling rule is deterministic, order-independent and documented, because three implementations (CPU, GPU, reference) must agree.
- Incremental update and full rebuild produce identical results, enforced by property tests.
- Empty space stays empty at every level, and a solid region never appears empty at a coarser level.
- Mip data is derived and never the source of truth; it is never serialized as authoritative content.
