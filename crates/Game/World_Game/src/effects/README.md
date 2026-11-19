# Effects

## Purpose

The effects domain is the gameplay side of visual feedback for destruction and impact: dust, debris particles and the small cues that make a hit feel like a hit. It decides when and where an effect happens and with what parameters; the drawing is done by the render layer.

## Scope

- Listening to hit, break and collapse events and spawning effect requests at the right place, scaled by the volume of voxels removed and the material involved.
- Dust clouds and debris particles: emitter parameters (count, velocity spread, lifetime, color taken from the broken voxels' materials), including the particles that replace small islands.
- Impact feedback: screen shake, hit flashes, crack or scorch marks on surfaces as data, muzzle and tool effects.
- Effect budgets: per-frame and total particle caps, with priority rules for dropping the least important effects.
- Effect definitions as hot-reloadable data.

## Out of scope

- GPU particle simulation and drawing belong in World_Render.
- Deciding that a small island becomes particles instead of a body belongs in World_Physics.
- Sounds belong in `audio`.
- Persistent changes to voxels belong in `interaction` and World_Core, never in effects.

## Relationships

Consumes events from `interaction`, `tools` and `creatures`. Uses material colors from World_Core's table. Hands requests to World_Render's particle interface. Camera effects apply through `player`.

## Rules

- Effects are cosmetic: no effect may change world state or physics outcomes.
- Every effect type has a budget and degrades gracefully, so a large collapse cannot stall a frame.
- Parameters come from material data and assets, not constants.
- Systems stay in `Update` and avoid changing live component layouts.
