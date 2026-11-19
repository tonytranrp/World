# Golden Command

## Purpose

This folder implements `cargo xtask golden --update`, the deliberate way to regenerate golden images and hashes. Reference files must not change by accident, so regeneration is a separate, explicit command that shows what it changed, not a side effect of a failing test.

## Scope

- Running the golden-producing tests with `UPDATE_GOLDEN=1`, so they rewrite their references in the fixture folders.
- Narrowing a run to chosen suites, scenes or file name patterns.
- Using the software adapter by default, so regenerated images are the reference class.
- Reporting what changed: files added, modified and removed, and for images the size of the difference, so a reviewer sees the scope before committing.
- Confirming that regenerated output is stable by running the producing tests again without update mode.

## Out of scope

- The comparison logic itself: the golden module of World_Testkit.
- Staging or committing the result. That is the reviewer's step.
- Deciding whether a change is acceptable.
- Running the normal test profiles: the test command.

## Relationships

Called through the Xtask dispatcher and built on its shared subprocess helpers. Runs the root `tests/` crate and any crate that uses the golden module. Writes only to the fixture locations that the root tests crate's path helpers define.

## Rules

- Updates are explicit. Nothing is rewritten unless `--update` is passed.
- The command never commits, stages, loosens a tolerance or edits test code to make a difference disappear.
- Reports list files in a stable order, so two runs are comparable.
- Output that fails the stability check makes the command fail and names the unstable files.
