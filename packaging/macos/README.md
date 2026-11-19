# macOS Packaging

## Purpose

This folder holds the metadata used to package the game for macOS. The platform needs two CPU architectures: `x86_64-apple-darwin` for Intel Macs (the performance-floor laptop) and `aarch64-apple-darwin` for Apple Silicon (the presentation machine and all newer Macs). Packaging combines them into one distributable application.

## Scope

- The packaging pipeline's inputs: both architecture builds are merged with `lipo` into a universal binary, wrapped into a `.app` bundle and then a `.dmg` using cargo-bundle or cargo-packager.
- Bundle metadata: display name, bundle identifier, minimum macOS version, category, high-resolution icon set and the `Info.plist` fields that cannot be derived.
- Entitlements and hardened-runtime settings needed for signing.
- Signing configuration: ad-hoc signing for the developer's own Macs (first launch uses right-click then Open), and Developer ID signing plus notarization for public sharing, which macOS requires.
- Layout of the bundle: where the binary and the copied `assets/` tree sit inside the `.app`.
- Notes on the Intel build's lifetime, which depends on GitHub keeping Intel macOS runners available.

## Out of scope

- Executing any of the above. The build and bundle logic is in `tools/Xtask`, triggered by CI jobs in `.github/workflows`.
- Cross-compiling from Windows. It would require Apple's SDK, which is licensed for Apple hardware, so it is not used.
- Credentials: certificates, Developer ID identity, app-specific passwords and notarization profiles are stored in CI secrets or the keychain, never here.
- Windows packaging (`../windows`).

## Relationships

- Consumed by `tools/Xtask`; the macOS build jobs in `.github/workflows` build both architectures.
- Bundles the `app/` binary with `assets/`.

## Rules

- No secrets, certificates or private keys in this folder.
- Version numbers and identifiers are derived from the workspace, not duplicated.
- Every release must be tested on the Intel and the Apple Silicon machine before it is considered shippable.
- Icons and other binary resources are tracked through Git LFS.
