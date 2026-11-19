# Scripts

## Purpose

This folder holds small shell and PowerShell helpers for developers and CI: one-step conveniences that are too trivial to justify a Rust implementation. The rule of thumb is that a helper which is a few lines of command invocation belongs here, and anything needing real logic belongs in `tools/Xtask`, where it is typed, tested and cross-platform.

## Scope

- Developer setup: installing the toolchain pieces, enabling Git LFS, installing optional tools such as the dioxus CLI for hot patching or alternative linkers.
- Launch helpers: running the game with commonly used flags, such as a selected backend, the `--mac-floor` mode or the hot-patch workflow.
- Thin wrappers that give CI and humans the same entry point for a command, such as running the render tests on software Vulkan.
- Environment setup snippets, such as setting `WGPU_BACKEND`.
- Each helper exists in the shell form the platform needs: PowerShell for Windows and POSIX shell for macOS and Linux.

## Out of scope

- Anything with branching logic, parsing, file generation or non-trivial error handling. That goes in `tools/Xtask`.
- Shader constant generation, asset baking, packaging and benchmarking, which are Xtask or the Bake tools.
- CI definitions, which are in `.github/workflows`.
- Application code and tests.
- Secrets, tokens and machine-specific paths.

## Relationships

- Calls `cargo`, `tools/Xtask` and the built binary; never contains logic the Xtask also needs.
- Used by developers and may be called from the jobs in `.github/workflows`.
- Nothing in the crates depends on these scripts.

## Rules

- Each script does one thing, is safe to run repeatedly and states what it does at the top.
- Scripts take the repository root from their own location, not from the caller's working directory.
- When a script grows logic, move it to Xtask and leave a thin wrapper or remove it.
- Provide a PowerShell and a shell variant for anything a Windows or Mac developer would run.
- Never embed credentials or absolute machine paths.
