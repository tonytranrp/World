# destruction

## Purpose

This category folder owns the full destruction pipeline: from damage being dealt, through voxels being removed or weakened, to the world being split into independent pieces and the smallest pieces being turned into particles. Neither Rapier nor Avian does fracture, so this layer is the game's own.

## Scope

The pipeline runs in stages, one subfolder per stage:

1. damage: applying impacts, explosions, and tool strikes to voxels, using material strength, and removing or weakening voxels through World_Core's edit API.
2. islands: after an edit, running union-find or flood fill over the edited object to find disconnected components of voxels.
3. anchors: for terrain, deciding which components still connect to an anchor such as bedrock; those that do not are released to fall.
4. debris: spawning a new rigid body for each sizable island, and converting islands under about 64 voxels into GPU particles that then sleep on arrival.

This level also holds the shared types that tie the stages together: damage events, the queue of pending detection work, and the per-frame voxel budget. All detection and split work is time-sliced to N voxels per frame, so a large collapse spreads across frames instead of stalling one.

## Out of scope

- Collider and mass construction for new bodies: colliders/dynamic.
- Rendering of debris and dust: World_Render.
- Long-running structural simulation such as stress propagation and fire: World_Sim, which builds on island detection from here.
- Engine calls: rapier.
- Deciding what causes damage: World_Game and the contact events from rapier.

## Relationships

Depends on colliders, rapier, and World_Core. Depended on by World_Sim (island and anchor results) and World_Game. The four subfolders are ordered stages; each depends only on the stages before it.

## Rules

- Voxels change only through the World_Core edit API, never by writing to the tree directly.
- Active bodies stay under the hard cap of about 500; when the cap is hit, new tiny pieces degrade to particles.
- Every stage is incremental and resumable, so work can pause when the budget is spent.
- Results must be reproducible for a given edit sequence, so regression scenes can hash them.
