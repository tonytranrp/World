# Devlog

## Purpose

This folder is the chronological record of development. Entries capture what was tried, what happened, what was learned and what comes next, in the order it occurred. Where plans and architecture pages describe the intended end state, the devlog preserves the path taken, including dead ends, which is often the most useful information when a similar problem returns.

## Scope

- Dated entries about work sessions or milestones: experiments run, measurements taken, bugs chased and their causes, and surprises.
- Performance notes: frame-time observations on the development PC and the Mac targets, brick-cache behavior, physics step cost, with enough context (scene, settings, hardware) to be comparable later.
- Reasons for abandoning an approach, written while the details are fresh, which may later be promoted into a decision record.
- Short notes pointing at screenshots, captures or test artifacts that illustrate a result.

## Out of scope

- Final decisions. Those go in `../decisions` as formal records.
- Plans and the build-order backlog. Those live in `../plans`.
- Stable explanations of how the system works. Those belong in `../architecture`.
- Research on outside sources. Those belong in `../research`.
- Large media files; entries link to or embed small images only, and bulk captures are kept elsewhere.

## Relationships

- Entries may link to decision records, plan sections, research notes and test scenes under `tests/`.
- Findings that prove durable are promoted into `../architecture` or `../decisions`; the devlog entry stays as the history.
- It depends on nothing in the codebase and nothing depends on it.

## Rules

- Entries are append-only and ordered by date; the file name starts with the date so listings sort correctly.
- Never edit an old entry to change history. Add a new entry that corrects it.
- State measurements with their conditions (machine, backend, scene, resolution, settings).
- Keep each entry short and specific; one topic per entry.
