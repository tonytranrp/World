# World_Cpu_Ref Golden Images

## Purpose

This folder holds the reference images of the CPU reference renderer's named test scenes. Each image is the accepted, human-reviewed rendering of one scene from one camera, so a change in traversal, camera conventions or shading shows up as a pixel difference rather than going unnoticed. These images are also what GPU comparison work can look at when a scene needs a visual reference.

## Scope

- Lossless PNG renders of the scenes built in the sibling test files, produced by this crate's own shading.
- One image per scene and camera, named after the scene.

## Out of scope

- Golden images of full Bevy frames and GPU renders: the workspace `tests/golden` folder.
- Exact hit data: tests compare hits against the dense DDA reference directly, not through images.
- Throwaway renders, such as the generated-terrain picture, which go to the build's scratch folder.

## Relationships

Written and read by the tests in the parent folder through World_Testkit's golden helpers. Rendering is pure IEEE arithmetic, so the images compare exactly on every platform.

## Rules

- An image changes only with `UPDATE_GOLDEN=1` (`cargo xtask golden --update` sets it), after reviewing the new render, with the reason in the commit message.
- A missing image fails its test; it is never created silently.
- Images are stored through Git LFS.
- Keep resolutions small.
