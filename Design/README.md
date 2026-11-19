# Design

## Purpose

Visual design for World: concept images that show how the game should look,
before the renderer can show it for real. They set the target for art
direction (painterly, saturated per-voxel colour, soft bounce light, fog) and
for the feel of 1/64 m voxels.

## Scope

- `concepts/`: AI-generated concept images, one file per scene and model,
  named `<scene>__<model>.png`, with the exact prompts in `prompts.json`.
- `generate_concepts.py`: regenerates them through an OpenAI-compatible image
  API. The key comes from the `SI_KEY` environment variable and is never
  written to a file.

## Out of scope

- Runtime assets the game loads: `assets/`.
- Golden images for tests: `tests/golden`.

## Rules

- Concept images are a mood target, not a spec. Image models draw the voxels
  two to three times too big: a real 1.56 cm voxel is the size of a sugar cube,
  a 1.8 m player is 115 voxels tall and a hand is about 12 voxels long, so the
  game will look finer and smoother than these images up close.
- Images are stored with Git LFS (`*.png` in `.gitattributes`).
