# Tools

## Purpose

The tools domain defines what the player holds and does with it: hand tools, held items, explosives, and the cutting, digging and destroying tools that make the sandbox work. It owns the behavior of each tool, from equip to use to cooldown, and nothing about how a hit becomes a voxel edit.

## Scope

- Tool and item definitions as data: shape of effect, reach, strength, rate of use, ammunition or charges, loaded from RON assets so they hot-reload.
- Tool state machines: equipped, idle, charging, firing, recharging.
- Held-item presentation: the voxel parts the player holds, which may use a finer voxel size than terrain because they fill the screen, plus sway and recoil requests passed to the player and animation layers.
- Explosive behavior: fuses, triggers and blast parameters (radius, impulse, damage falloff), emitted as damage requests.
- Tool selection and inventory slots.

## Out of scope

- Ray picking, targeting, damage resolution and edit application belong in `interaction`.
- Particle and sound responses belong in `effects` and `audio`; tools only emit events.
- Voxel edit primitives belong in World_Core; fracture and island splitting belong in World_Physics.
- Creative-mode spawning menus belong in `sandbox` and `ui`.

## Relationships

Reads player state from `player` (eye position, look direction, held slot). Sends damage and edit requests to `interaction`. Emits events that `effects` and `audio` consume. Uses World_Anim for the held-part rigs.

## Rules

- A tool never mutates the world directly; it produces requests that `interaction` validates and applies.
- New tools are data first: prefer a new asset entry over new code, and add code only for new behavior kinds.
- Damage values reference the material table (strength, density), never hard-coded per-material numbers.
- Keep systems in `Update` and avoid changing live component layouts for hot patching.
