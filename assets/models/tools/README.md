# Tool Models

## Purpose

This folder holds the voxel models of items the player holds and uses: digging and breaking tools, placement tools and similar hand-held equipment. Held items fill the screen in first person, so they get special treatment: they may be authored at a finer scale than the world, such as 1/128 m, to hold up under close inspection.

## Scope

- `.vox` sources and baked grids for hand-held tools.
- Metadata for each tool: voxel scale, grip pivot, attachment bone (normally a hand bone of the player rig), and the muzzle or impact point used for effects and hit tests.
- Variants that share a grip and attachment, such as different materials or upgrade tiers.

## Out of scope

- The player's own body parts. Those go in `../player`.
- Tool behavior: damage, reach, swing timing and what a tool does to voxels. That is code in `World_Game` and data in `assets/config`.
- Sounds for tools. Those go in `assets/audio`.
- World props, even if a prop can be picked up. Those go in `../props`.

## Relationships

- Attached to a hand bone defined in `assets/rigs`; loaded by `World_Anim` and `World_Game`.
- Drawn by the box-march pass in `assets/shaders/passes/dynamic_box`, and also visible in shadow and reflection passes.
- Baked by `tools/Bake_Vox`.
- Materials resolve against `assets/config`.

## Rules

- Source and baked files are tracked through Git LFS.
- Scale is explicit in metadata; a finer-scale tool must never be assumed to share the world's voxel size.
- The grip pivot is part of the data, so swapping a tool never requires code changes.
- Keep grids small: a tool is a dense grid that is rendered every frame while held.
