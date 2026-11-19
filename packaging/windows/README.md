# Windows Packaging

## Purpose

This folder holds the metadata used to package the game for Windows, built for the `x86_64-pc-windows-msvc` target. Windows is the development platform, so this packaging path also doubles as the way test builds are shared between machines.

## Scope

- Executable metadata embedded at link time: product name, company, description, version resource fields and the application icon.
- Installer or archive settings: install layout, shortcuts, uninstall information and the choice of an installer package versus a portable archive.
- Layout of the distributable: where the executable and the copied `assets/` tree sit relative to each other, so the asset server finds content with no configuration.
- Signing configuration for Windows code signing when a certificate is available, referenced by name only.
- Runtime prerequisites that the package must carry or declare, such as graphics-runtime requirements; the game selects DX12 or Vulkan itself at startup through wgpu.

## Out of scope

- The build and packaging logic. That is code in `tools/Xtask`, driven by CI in `.github/workflows`.
- Game assets. They live in `assets/` and are copied into the package.
- Secrets: signing certificates and passwords stay in CI secrets or the developer's certificate store.
- macOS packaging, which is in `../macos`.
- Developer convenience scripts for running the game, which belong in `scripts/`.

## Relationships

- Read by `tools/Xtask` and by Windows release steps in `.github/workflows`.
- Packages the `app/` binary together with `assets/`.
- Nothing in the game depends on this folder at runtime.

## Rules

- No secrets or certificates in the repository.
- Metadata that duplicates workspace information (version, name) is read from the workspace manifest, not copied.
- The package must run from any install path, with no absolute paths.
- Binary resources such as icons and installer images use Git LFS.
