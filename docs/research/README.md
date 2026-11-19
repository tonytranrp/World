# Research

## Purpose

This folder holds the project's notes on outside work: reference projects, papers, articles and technical talks that inform the engine. The goal is to capture what was learned once, with sources, so the reasoning behind a design does not depend on someone remembering or re-reading it.

## Scope

- Notes on reference projects: Octo (Rust compute ray marching of voxels, native and WebGPU), Teardown (dense per-object voxel volumes and its destruction approach), tree64 and VoxelHex (Rust 64-tree implementations), voxelis (SVO-DAG), and similar.
- Notes on techniques and papers: GigaVoxels and ray-guided brick streaming, sparse 64-tree traversal, SVDAGs, shell-mesh voxel animation, radiance caches, temporal upscaling and denoising.
- Benchmark findings from third parties, with a note on how well they apply to this workload.
- Capability research: hardware and backend features, wgpu limits, and platform constraints.
- For each note: what the source is, what is relevant to this project, what was adopted, rejected or left open, and links.

## Out of scope

- Choices made as a result. Those are records in `../decisions`.
- Plans built from the findings. Those belong in `../plans`.
- Explanations of this project's own architecture. Those belong in `../architecture`.
- Copied source code, copied text from papers or articles, and binary downloads. Notes are summaries in the author's own words with links.
- Experiments of our own with results; measurements about this engine are recorded in the devlog or in test and benchmark outputs.

## Relationships

- Feeds `../plans` and `../decisions`, which cite notes here as evidence.
- Architecture pages link here for background.
- Crates that adapt an outside idea may reference the relevant note in their module docs.

## Rules

- Every note cites its sources and says what was verified and what was not.
- Mark claims as third-party, unverified or measured by us so confidence is clear.
- One subject per note, named for the subject.
- Respect licenses: summarize and link, never reproduce.
- Notes record what was true when written; later contradictory findings are added as new notes or clearly marked corrections, not silently overwritten.
