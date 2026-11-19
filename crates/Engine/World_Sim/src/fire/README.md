# fire

## Purpose

This folder simulates combustion: fire starting, spreading across flammable voxels, burning for a time, and leaving something behind. Behavior is driven by flammability and related values from the World_Core material table, so adding a burnable material is a data change.

## Scope

- Per-voxel fire state: unburnt, igniting, burning, and burnt out, with heat or fuel remaining for materials that burn over time.
- Ignition: from explicit sources (torches, explosions, lava, emissive hot materials) and from neighboring fire, with probability and heat thresholds from the material table.
- Spread: neighbor checks with direction bias, such as upward spread, and a stochastic step using a deterministic generator.
- Burn products: replacing burnt voxels with ash, charcoal, or empty space through the edit API, and releasing smoke and ember effect data.
- Damage to structure: burning materials lose strength, which feeds stress and destruction, so burned supports can collapse.
- Extinguishing: running out of fuel, lack of adjacent air, and water contact.
- Light and effect hints: emission data for burning voxels so the renderer's lighting can pick them up as emissive.

## Out of scope

- Heat or fire effects on bodies other than voxels: gameplay in World_Game.
- Collapse mechanics: stress and World_Physics/destruction.
- Water and fluids: fluids.
- Drawing flames and smoke: World_Render.

## Relationships

Depends on World_Core (materials, edit API, tree access), World_Physics (damage events), and budget. Depended on by the crate's schedule. It communicates with stress and fluids only through voxel and material state.

## Rules

- Flammability, burn time, and products come only from the material table.
- Work is limited to the active burning region and bounded by the shared budget.
- Spread is deterministic for a fixed seed.
- Burning never writes the tree directly; it submits edits.
