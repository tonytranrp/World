# Packaging

## Purpose

This folder holds the platform-specific metadata needed to turn a built game into something a person can install and run: bundle descriptions, icons, installer settings, signing configuration and platform manifests. It is metadata only. The logic that builds, merges, bundles and signs is code in `tools/Xtask`, which reads what is stored here.

The project ships on two platforms: Windows (target `x86_64-pc-windows-msvc`) and macOS, where an Intel build and an Apple Silicon build are produced and merged into one universal binary. Each platform has its own subfolder so one platform's needs never leak into another.

## Scope

- `macos/`: bundle metadata for the `.app` and `.dmg`, entitlements, icon sets and signing settings for cargo-bundle or cargo-packager.
- `windows/`: executable metadata, icons, and installer or archive settings for the Windows build.
- Cross-platform packaging inputs such as product name, identifiers and license text, kept in the subfolder that uses them or in a clearly shared location agreed in `tools/Xtask`.
- Notes on which items in the bundle are copied from `assets/` and what the bundle layout must look like.

## Out of scope

- Build, merge, bundle and signing procedures. Those are implemented in `tools/Xtask` and invoked from CI in `.github/workflows`.
- Game assets themselves. Those live in `assets/` and are copied into the bundle.
- Secrets: signing certificates, Apple Developer credentials and notarization passwords. They are stored in CI secrets or the developer's keychain, never in the repository.
- Crate Cargo manifests; those stay with their crates and the workspace root.
- Small developer helper scripts, which belong in `scripts/`.

## Relationships

- Read by `tools/Xtask` and by the release jobs in `.github/workflows`.
- Bundles the output of the `app/` binary together with the `assets/` tree.
- Depends on nothing at runtime; nothing in the game depends on it.

## Rules

- No secrets, ever.
- Everything here is declarative and reproducible: the same inputs produce the same package.
- Metadata that must match code (application identifier, version) is read from the workspace manifest by Xtask rather than duplicated here.
- Large binary resources (icons, installer images) use Git LFS.
