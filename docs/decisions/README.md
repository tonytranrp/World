# Decisions

## Purpose

This folder is the project's decision log. Each significant choice, such as the physics engine, the voxel size, the tree structure or the platform floor, gets one short record that captures why it was made. The log lets a future reader (or the same person a year later) understand the reasoning without reconstructing it, and see what was ruled out and why.

## Scope

- One record per significant decision, short enough to read in a couple of minutes.
- Each record states: the context and constraints at the time, the choice made, the alternatives considered and why they lost, and the consequences, including costs accepted and follow-up obligations.
- Records for reversals: when a decision changes, a new record states what it supersedes and why.
- Decisions that cross crate boundaries or constrain future work: dependency pins and upgrade policy, data format choices, determinism rules, and similar.

## Out of scope

- Long-form planning and option surveys. Those belong in `../plans`; a record summarizes the outcome and links back.
- Explanations of how the final system works. Those belong in `../architecture`.
- Notes on outside projects. Those belong in `../research`.
- Small implementation choices that a code comment or review can carry.
- Day-to-day progress notes, which go in `../devlog`.

## Relationships

- Records are fed by plans and research and are referenced from architecture pages and, where useful, from code comments.
- Records constrain the crates: for example the rule that physics engine types stay confined to one crate comes from a decision here.
- A record outranks any plan statement that disagrees with it.

## Rules

- Records are append-only. Once accepted, a record is never rewritten; only its status line may change to show it was superseded, with a link to the superseding record.
- Records are numbered in sequence so the order is unambiguous, with a short descriptive title in the name.
- Every record has a status (proposed, accepted, superseded) and a date in its own text.
- Keep each record to one decision; split bundles.
- State consequences honestly, including the downsides.
