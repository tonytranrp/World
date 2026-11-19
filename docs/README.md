# Docs

## Purpose

This folder is the written memory of the project. It holds the long-form plan, the record of decisions and their reasons, explanations of how the system fits together, notes on outside work the project learns from, and a chronological development log. The root README says what the project is; this tree explains why it is built the way it is and how the parts connect. This page is an orientation index: read it to decide which subfolder to open or write into.

## Scope

- `plans/`: long-form plans, starting with the engine plan. Plans are living references that are revised as understanding improves.
- `decisions/`: one short record per significant decision (context, choice, alternatives, consequences). Accepted records are never rewritten; a changed decision is a new record that supersedes the old one.
- `architecture/`: explanations of how the system fits together: the crate graph, the data flow from the CPU tree to the GPU, the frame build order and the physics loop.
- `research/`: notes on reference projects and papers, such as Octo, Teardown, tree64 and GigaVoxels, including what was learned and what was adopted or rejected.
- `devlog/`: dated development notes in chronological order.

## Out of scope

- Per-folder explanations of what a folder is for. Each folder explains itself with its own README, and docs does not duplicate them.
- API documentation. That is generated from Rust doc comments in each crate.
- Task tracking and issue lists. Those belong in the issue tracker; the plan carries only the build-order backlog.
- Generated binary files, large captures and assets. Those go in `assets/` or `tests/`, and only links or small images belong here.

## Relationships

- Documents describe the crates under `crates/`, the tools under `tools/` and the content under `assets/`; they do not depend on them at build time.
- Decisions are referenced from plans and architecture pages, and architecture pages reference research.
- Code comments may point to a decision record when a choice needs its rationale.

## Rules

- Write for a reader who joins later: state context, not just conclusions.
- Prefer one topic per page, with a descriptive file name in lowercase with hyphens.
- Decision records are append-only; supersede, do not edit.
- Link sources and external references so a claim can be checked.
- Keep explanations in architecture consistent with the plan, and fix the lower-authority page when they diverge.
