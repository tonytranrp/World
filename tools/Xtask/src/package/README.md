# Package Command

## Purpose

This folder implements the command that turns built binaries and assets into distributable packages for Windows and macOS. It encodes the release steps so they are repeatable on a developer machine and on CI runners.

## Scope

- macOS: building the `x86_64-apple-darwin` and `aarch64-apple-darwin` targets, merging them with `lipo` into one universal binary, assembling the `.app` bundle (binary, assets folder, bundle metadata and icon), producing a `.dmg`, and ad-hoc signing. Optionally accepts a Developer ID and notarization inputs when supplied externally.
- Windows: building the `x86_64-pc-windows-msvc` target and assembling the folder or archive that holds the executable and assets.
- Collecting runtime assets (shaders, material tables, worldgen parameters, baked content) and excluding development-only files.
- Validating the result: expected files present, universal binary contains both architectures, signature verifies.
- Clear errors when a step needs a platform the command is not running on.

## Out of scope

- Compiling game code beyond invoking cargo with the right targets and profiles.
- Cross-compiling macOS targets from Windows, which is not supported.
- Release publishing, update distribution and store submissions.
- Asset baking, which belongs to the baking tools.

## Relationships

Called through the Xtask dispatcher. Invokes cargo and platform tools (`lipo`, disk image and signing utilities). Packages the output of the application crate together with the assets folder.

## Rules

- Packaging is reproducible from a clean checkout.
- Only the game and its runtime assets go into a package; tools, tests and sources never do.
- Platform-specific steps are gated and fail fast elsewhere.
- Signing credentials are supplied by the environment and never stored in the repository.
