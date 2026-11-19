# World_Cpu_Ref

## Purpose

World_Cpu_Ref is a slow, exact CPU ray tracer for the voxel world. It exists to be ground truth. The GPU renderer is complex, parallel and hard to debug; this crate renders the same small scenes the simple, obvious way so that the GPU's output can be compared against something trustworthy. When the GPU and this crate disagree, the GPU is wrong until proven otherwise.

## Scope

- Pinhole camera and per-pixel primary ray generation that matches the GPU's ray setup, including internal-resolution handling.
- A deliberately simple traversal of the 64-tree, written for clarity and exactness rather than speed, including level-limited traversal that mirrors how distant rays stop at coarse mip levels.
- A reference G-buffer: depth, normal, material id and motion vectors per pixel, in the same conventions the GPU pass writes.
- Reference lighting queries for the early lighting stages: sun shadow rays and short ambient-occlusion rays.
- Output and comparison utilities: writing a rendered test scene to a PNG, exact per-pixel hit comparison, tolerance comparison for shaded images, and difference visualization.
- A small catalog of test scenes built through World_Core, such as planes, steps, spheres and thin walls that stress boundary cases.

## Out of scope

- Production CPU queries used by gameplay and physics: the ray module of World_Core.
- Real-time or parallel rendering performance; speed is a non-goal.
- Anything that opens a window, talks to a GPU or uses Bevy.
- Golden-image tests for the full Bevy frame: World_Render.
- Temporal accumulation, denoising, upscaling and global illumination caches.

## Relationships

It depends on World_Core for the tree, coordinates and materials, and may use World_Gen as a development dependency to build scenes. World_Render's tests depend on it to verify that GPU hits match exactly on small scenes. Nothing in Foundation depends on it.

## Rules

- Correctness over speed. If an optimization would make the code harder to audit, do not apply it.
- Hit results are compared exactly, not within tolerance; shaded results use a stated, documented tolerance.
- The reference does not share traversal code with World_Core's production path, so one bug cannot hide in both.
- Conventions (axes, handedness, depth encoding, pixel centers) are documented once and match the GPU.
- Output is deterministic and independent of thread count.
