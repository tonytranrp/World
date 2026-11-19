# CLI

## Purpose

This folder defines the program's command-line interface: the flags and arguments the player or developer can pass, how they are parsed, and the typed overrides they produce. It turns text into a validated structure and does nothing else.

## Scope

- The argument definitions: `--backend` (dx12, vulkan, metal), `--mac-floor`, and developer flags such as scenario selection, logging level, and enabling inspector or profiling features.
- Parsing and validation, including mapping a backend name to the wgpu backend set and rejecting a backend that the current operating system cannot offer.
- Reading related environment variables, notably `WGPU_BACKEND`, and combining them with flags.
- Help and version output, and clear error messages for malformed arguments.
- The typed override structure handed to startup.

## Out of scope

- Persisted user preferences belong in `settings`.
- Acting on the arguments (creating the device, applying mac-floor limits, registering plugins) belongs in `startup`.
- In-game commands and consoles belong in World_Game's ui domain.

## Relationships

Depends only on an argument-parsing library and standard types. Its output is consumed by `settings`, which merges overrides over stored values, and by `startup`.

## Rules

- Parsing has no side effects: no logging initialization, no device creation, no file writes.
- Every flag has a documented default and a corresponding settings entry where it makes sense.
- Flags map to the same configuration fields the settings file uses, so there is one definition of each option.
- Unknown or invalid input fails early with a message naming the flag.
- Adding a flag means adding it to the shared configuration, not special-casing it in startup.
