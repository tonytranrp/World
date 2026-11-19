# Creatures

## Purpose

The creatures domain holds NPCs and animals: their spawning, behavior and AI. Creatures reuse the same rig system as the local player, so a creature is a set of rigid voxel parts driven by animation, IK and, when hit, a physics ragdoll. This folder supplies the decisions; the machinery for moving the bodies lives in the engine crates.

## Scope

- Creature definitions as data: rig reference, size, materials, health, movement capabilities, senses.
- Behavior: state machines or behavior trees for idle, wander, flee, chase and attack, plus goal selection.
- Perception: sight and hearing queries, using ray tests through the voxel tree.
- Navigation over voxel terrain: path requests and steering that cope with destructible, changing ground.
- Reactions to damage: triggering ragdoll through World_Anim, dropping limbs, death.
- Spawning rules and population limits.

## Out of scope

- Rig definitions, IK, procedural gait and the ragdoll mapping belong in World_Anim.
- Rigid bodies and collision belong in World_Physics.
- Rendering of creature parts belongs in World_Render, which treats them as dynamic objects.
- Player-specific logic belongs in `player`.

## Relationships

Depends on World_Anim, World_Physics and World_Core. Receives damage through `interaction`. Emits events for `audio` and `effects`. May be spawned by `sandbox` scenarios.

## Rules

- Behavior is separate from animation: AI outputs intents, and the rig layer turns them into motion.
- Active body counts share the global cap, so population and ragdoll limits must respect it.
- Perception and navigation use bounded work per frame.
- Creature parameters are data assets that hot-reload.
- Systems run in `Update` with stable component layouts.
