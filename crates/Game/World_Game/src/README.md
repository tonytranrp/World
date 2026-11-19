# World_Game Source

## Purpose

This folder is the source tree of the gameplay crate. It holds the crate root, which declares the modules, builds the single game Plugin and defines the prelude, plus one subfolder per gameplay domain. It is where the structure of the game is decided, so the layout here mirrors how a player experiences it.

## Scope

- The crate root: module declarations, the top-level Plugin that registers each domain's sub-plugin in a defined order, and the prelude of public types.
- Small cross-domain items shared by several domains, kept minimal: shared events, marker components and system-set labels used to order domains.
- The domain folders: player, tools, interaction, sandbox, creatures, effects, audio, ui and save. New gameplay domains get a new snake_case folder here, each with its own sub-plugin and README.

## Out of scope

- Anything that is not gameplay. Engine-level mechanisms (rendering, physics stepping, voxel storage) live in the Engine and Foundation crates.
- Loose gameplay code placed directly in this folder. If it has a home in a domain folder, it goes there; if it is shared by three or more domains, consider whether it belongs in an Engine crate.
- Tests, which live in the crate's tests folder.

## Relationships

Depends on the Foundation and Engine crates through their plugins and preludes. Dependents see only what the prelude exports; the application binary adds the Plugin. Domains inside depend on each other only through events, shared components and system sets declared at this level, so no domain folder imports another's private modules.

## Rules

- One top-level Plugin per crate, one sub-plugin per domain folder.
- Keep the shared surface at this level small, since everything here is visible to every domain.
- Systems stay in `Update` so they hot-patch; avoid changing the layout of live components and resources.
- Domain ordering is expressed with system sets declared here, not with implicit assumptions about registration order.
- Module folders are snake_case and named for a gameplay concept, not for a technical layer.
