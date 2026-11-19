# Xtask Source

## Purpose

The source tree of the automation runner. It holds the entry point that parses the requested command and dispatches to one of the command modules, plus the small amount of plumbing every command shares. Each command is a snake_case subfolder, so adding automation means adding a folder, not growing a single file.

## Scope

- The entry point: argument parsing, selecting a command, translating its result into an exit code.
- Shared helpers: finding the workspace root, running and streaming subprocesses (cargo, rustc, lipo, hdiutil, codesign), working with paths, and uniform error and log formatting.
- Command folders: `codegen`, `bench` and `package`, each self-contained with its own README.

## Out of scope

- The logic of any one command, which lives in that command's folder.
- Engine or game logic.
- Format definitions, which belong in World_Core.

## Relationships

Depends on World_Core where commands need shared constants. Command folders depend on the helpers here, never on each other. Nothing outside the crate depends on this folder.

## Rules

- A command folder owns its behavior and options; the entry point stays a thin dispatcher.
- Shared helpers stay small and generic. Command-specific knowledge never moves into them.
- Every command is safe to run repeatedly and from any working directory inside the workspace.
- Failures return nonzero exit codes with an explanation, so CI can gate on them.
