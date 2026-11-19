# Plans

## Purpose

This folder holds the long-form plans for the project: documents that set direction, break work into an order and record the reasoning that ties many choices together. The engine plan is the central one: it covers rendering, voxel data structures, physics, animation, procedural generation, hot reload, testing, workspace layout, macOS packaging, the build-order to-do list and risks.

## Scope

- The engine plan and later plans for additional areas (for example a gameplay plan or a content plan).
- For each plan: goals, constraints and hardware targets, the options considered with a ranked verdict, the chosen approach, the build order with a definition of done per task, risks with mitigations, and open questions.
- Revisions to a plan when understanding improves. A plan is a living reference and may be edited in place, since it states the current best understanding.

## Out of scope

- Individual decisions that must stay fixed once accepted. Those are records in `../decisions`, where context, alternatives and consequences are preserved and never rewritten.
- Explanations of how the built system works. Those belong in `../architecture`.
- Notes on reference projects and papers. Those belong in `../research`.
- Chronological progress notes. Those belong in `../devlog`.
- Folder-by-folder structure documentation. Each folder describes itself in its own README.

## Relationships

- Plans draw on `../research` for evidence and produce `../decisions` when a choice is made final.
- `../architecture` documents the result once a plan has been carried out.
- Plans describe the crates in the Cargo workspace, the tools and the asset tree, and are the reference those folder READMEs stay consistent with.

## Rules

- Each plan states its scope up front and links the open questions that still gate it.
- Claims that depend on measurement are marked as targets to validate, not facts, until a measurement exists.
- When a plan conflicts with a decision record, the record wins, and the plan is corrected.
- Plans are written in plain prose with tables for comparisons; every ranked comparison states why the loser lost.
- Cite sources for external claims.
