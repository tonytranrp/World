# Composite

## Purpose

The last stage of the voxel renderer: it places the upscaled image into Bevy's view target and lets Bevy's own passes finish the frame. Atmosphere, UI, gizmos, particles and post effects composite here, over the voxel image, using the depth the voxel passes wrote.

## Scope

- Copying or blending the upscaled result into the main view target in the format Bevy's later passes expect.
- Hand-off conditions for Bevy passes: HDR target format, tone mapping and exposure ordering, and bloom or other post effects that expect scene-referred color.
- Atmosphere and fog integration: applying distance fog and sky so that coarse far mips blend invisibly into the horizon, using depth from the G-buffer.
- Ordering with rasterized content: transparency, gizmos, particles for dust and tiny debris, and UI, all depth-tested against the depth texture written by `primary` and `dynamic_box`.
- Output color space handling for the presented surface.

## Out of scope

- Producing, lighting or upscaling the image: the other pass folders.
- UI layout, particle simulation and post-effect definitions: Bevy and the game crates.
- Render-graph edge definitions: `graph`.

## Relationships

Depends on `upscale` for its input, on `gbuffer` depth, and on `graph` for placing it before Bevy's UI and post-processing nodes. World_Game's UI and effects are indirect dependents, since they draw after this stage.

## Rules

- Voxel output must be a normal Bevy view target as far as downstream passes can tell; they should need no knowledge of the ray tracing path.
- Depth consistency is mandatory: occlusion between voxel geometry and rasterized content must be correct.
- No lighting or accumulation logic lives here.
- The stage is identical on every tier.
