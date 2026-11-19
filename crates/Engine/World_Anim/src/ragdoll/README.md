# ragdoll

## Purpose

This folder turns an animated character into a physical one. Because each bone already owns a separate rigid voxel part, the ragdoll maps one-to-one: every part becomes a rigid body and every bone a joint. That same property is what lets limbs break off.

## Scope

- Ragdoll construction: for each part, requesting a body from World_Physics (with its voxel collider and mass from voxel density), and for each bone, a joint with the rig's limits and the bone's anchor points.
- Mode switching: animated to ragdoll when triggered (death, a heavy hit, a fall) and back to animated, with body velocities initialized from the animated motion at the moment of the switch.
- Blending: partial ragdoll, where some limbs are simulated and others stay animated, and a short blend when returning to animation.
- Dismemberment: breaking a joint when force exceeds its strength, releasing the part as an independent body, and informing parts to detach.
- Syncing: copying physics body transforms back onto bone Transforms each frame while simulated, and keeping bodies kinematic while animated if they must push the world.
- Self-collision filtering between adjacent parts.
- Spawning debris when a part is itself destroyed.

## Out of scope

- The physics engine and joints themselves: World_Physics.
- Part voxel data and attachment: parts.
- Rig definition and limits: rig.
- Deciding when characters die or get hit: World_Game.

## Relationships

Depends on rig, parts, and World_Physics (bodies, joints, dynamic colliders, destruction). Depended on by World_Game. Sibling of ik and procedural, which are suspended while ragdoll is active.

## Rules

- Exactly one body per part and one joint per bone.
- Joint limits and strengths come from the rig asset, not constants.
- This crate never names the physics engine; it uses World_Physics types only.
- The active-body budget applies: ragdolls count against the world cap.
