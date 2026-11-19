# Player

## Purpose

The player domain connects the local player to the engine: input, movement control, the body and the camera. It is the glue that turns key, mouse and gamepad state into motion for the character controller and keeps the first-person view attached to the body.

## Scope

- Input mapping: translating raw input into player intents (move, look, jump, use, switch tool) through an action layer that can be rebound.
- Controller glue: feeding intents to the kinematic capsule in World_Physics (move-and-slide, step-up of about 0.3 m, ground snapping) and reading back grounded state and velocity.
- The player entity: its body parts spawned from the rig defined in World_Anim, with the head hidden from the main view only so the full body still appears in shadows and reflections.
- Camera at the eyes: position, field of view, head-bob and recoil offsets applied as view-only transforms.
- Player state components such as health and held-tool slot, used by other domains.

## Out of scope

- The character controller algorithm itself belongs in World_Physics.
- Rig data, IK solving and ragdoll mapping belong in World_Anim.
- Tool behavior belongs in `tools`; picking and damage belong in `interaction`.
- Rendering of the body belongs in World_Render.

## Relationships

Depends on World_Physics for the controller, World_Anim for the rig and World_Render for the camera plugin. `tools` and `interaction` read player components such as eye position and look direction. `ui` reads player state for the HUD.

## Rules

- Systems run in `Update`; controller input is sampled per frame and consumed in the fixed step.
- The camera never drives physics; it follows the body with interpolation.
- View-only offsets (bob, recoil) must not feed back into collision.
- Avoid changing the layout of live player components; add new components instead.
