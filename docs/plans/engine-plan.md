# World — Micro-Voxel Physics Game: Engine Plan

Nov 18, 2026 · @tony

## Decisions at a glance

The plan: build on Bevy 0.20. The world is stored as a sparse voxel tree and drawn by tracing rays through it in compute shaders. Rapier handles physics through its built-in voxel colliders. Updated Nov 18 with your answers: Rapier, 1/64 m voxels, a fixed but large world, Windows plus a low-end Mac. GPU memory becomes a fixed brick cache filled by what rays actually hit, so the Mac floor holds whatever the world size.

| Area | Pick |
| --- | --- |
| Engine | Bevy 0.20, released Oct 8, 2026. Pin the exact version and upgrade once per Bevy release. |
| Graphics backends | Windows (DX12/Vulkan) and macOS (Metal), with wgpu picking the backend at runtime. The baseline renderer never requires hardware ray tracing or mesh shaders, so a low-end Mac runs it. |
| Voxel size | 1/64 m (1.56 cm), as you chose. Physics collides on a coarser 1/16 m grid. |
| World data | A sparse 64-tree (4×4×4 children per node) with palette-compressed materials. Small dense grids for dynamic objects. |
| Rendering | A compute pass traces rays through the tree into a G-buffer, and Bevy composites the result. Dynamic objects are boxes, and a shader marches rays inside each one. The world is never turned into triangle meshes. A fixed GPU brick pool holds only what rays requested. |
| Physics | Rapier through bevy\_rapier3d 0.37, as you chose. The Rapier validation scene in To-do checks it at Mac-class CPU budgets. |
| Destruction | Removing voxels triggers island detection, and each disconnected island becomes a new rigid body. Tiny debris becomes particles. |
| Animation | Each bone owns a rigid voxel part, driven by Bevy's AnimationGraph, plus IK and a physics ragdoll. |
| Procedural generation | A deterministic CPU core addressed by chunk. Props are built from SDF graphs, so they can be voxelized at any resolution. |
| Hot reload | The asset file watcher reloads shaders and configs. Bevy hotpatching (`dx serve --hot-patch`) patches systems while the game runs. |
| Testing | The core crate has no Bevy dependency and is covered by property tests. Headless App tests cover the ECS, and golden-image tests cover rendering. |

## How the game looks and renders

You play in first person inside a world where every surface is made of 1.56 cm voxels. Up close it reads as finely sculpted detail. From a few metres away it reads as solid, naturally lit 3D. The art direction is painterly, like John Lin's demos: saturated per-voxel color, soft bounce light and gentle fog, not photoreal materials.

What the player sees:

- **Up close (0–2 m).** Individual voxels show as tiny crisp cubes, each with its own color and material: stone grain, bark, soil. Your hands and held tools are voxel parts too, possibly at a finer 1/128 m, because they fill the screen.
- **Mid range (2–30 m).** Voxels blend into surfaces, and lighting does the work: sun shadows, ambient occlusion in crevices, bounce light.
- **Far (30 m to the horizon).** The same tree is drawn from coarser mip levels, and Bevy's atmosphere and fog hide the steps between detail levels.
- **Sky.** Bevy's procedural atmosphere.
- **Destruction.** Chunks break off along voxel boundaries, debris tumbles, and dust is drawn as particles.
- **Light.** One sun plus emissive voxels (lamps, lava) feed the GI cache. Nothing is baked, because everything can be destroyed.

**Reflections.** Full reflections are in scope, but how they look (mirror-sharp water, glossy floors, painterly blur) is still open. Reflection rays reuse the primary-ray traversal, so the voxel world reflects at low extra cost. The player's body must appear in them, so reflection rays also test dynamic-object boxes through a small GPU BVH. On the 2019 Mac, reflections trace at half resolution and only on smooth materials.

How one frame is built:

1. Stream in the bricks that last frame's rays asked for.
2. Trace primary rays in a compute pass at the internal resolution.
3. Draw dynamic objects, hands and tools with the box-march pass.
4. Light the frame: shadow rays, AO, GI cache.
5. Accumulate over time and upscale to the screen.
6. Draw UI and post effects.

Targets per machine (to validate, not measured):

|  | Windows PC (RTX 4070 Laptop): development | 2019 16-inch MacBook Pro: performance floor | M5 MacBook Air: presentation |
| --- | --- | --- | --- |
| Backend | DX12, Vulkan fallback | Metal | Metal |
| Internal resolution | 1440p | About 720p | About 1080p |
| Output | Native panel | 3072×1920, upscaled | Native panel, upscaled |
| Frame rate | 60+ fps | 60 fps goal, 30 fps floor | 60 fps, sustained through a 20-minute demo (the Air is fanless, so test for throttling) |
| Lighting | Painterly GI, plus optional hardware-RT extras and DLSS | Shadows, AO, GI cache updated less often | Full painterly GI. Apple added hardware RT with M3, so RT extras are optional here. |
| Brick pool | Up to 2 GB | Up to 1 GB | Up to 2 GB of shared memory |

## Architecture at a glance

&#91;embedded content: engine data flow · CPU store, physics loop, GPU frame\]

The CPU 64-tree is the single source of truth. Physics edits it and the GPU mirrors it. Dynamic objects take a separate box-march path into the same G-buffer.

## Rendering pipeline

A compute shader traces one ray per pixel through the 64-tree to find what's visible. It writes the result to a G-buffer, and Bevy's own passes composite with it.

1. **Upload.** The CPU tree is authoritative. Each frame, bricks that rays requested, plus edited ones, stream into a fixed GPU brick pool under an MB-per-frame budget. The least recently used bricks are evicted.
2. **Primary rays.** A compute pass runs per pixel, or per 8×8 tile with beam optimization. It writes depth, normal, material id and motion vectors. Depth also goes into Bevy's depth texture, so rasterized UI, gizmos and particles occlude correctly.
3. **Dynamic objects.** Each object is an oriented box. The box is rasterized, and its fragment shader marches the object's own dense grid in object space, the way Teardown draws objects. Results land in the same G-buffer.
4. **Lighting v1.** Sun shadow rays and short ambient-occlusion rays, traced through the same structure.
5. **Lighting v2.** Global illumination from a world-space radiance cache: irradiance probes per brick, updated incrementally, with temporal accumulation and denoising.
6. **Lighting v3 (optional, tier A).** Solari for any triangle content. DLSS-RR on NVIDIA through [dlss\_wgpu](https://bevy.org/news/bevy-0-20/).
7. **Bevy integration.** Custom render-graph nodes run in the Core3d graph, and the traversal code is shared across passes as WESL modules.

Why we don't mesh: greedy meshing at 3 cm produces millions of triangles per area and forces a remesh on every edit. Rasterization is a perfect primary-ray test, but only while voxel counts stay low. [Ray tracing scales to far higher voxel counts](https://hacker-news-clone.nuxt.dev/item/46285319). Bevy 0.20's mesh shaders [can draw voxels](https://bevy.org/news/bevy-0-20/), but they don't run on the web and need recent GPUs, so they stay a tier-A experiment.

Performance targets: 60 fps in \`--mac-floor\` mode (720p internal, tier B, capped pool) on the RTX 4070 Laptop. Full mode targets 1440p with tier A extras on the same PC.

## What your answers changed

Your answers lock in five things: Rapier, 1/64 m voxels, a large fixed world, single-player first person, and a 2019 16-inch MacBook Pro as the floor. That combination only works if GPU memory gets a fixed budget and on-screen voxels stream into it.

- **The voxelbee reference.** That [devlog](https://www.youtube.com/watch?v=i7vq-HY10hI) is a custom C++/Vulkan engine doing voxel octree ray casting, with a voxel cache tested down to 1 MB of RAM. It cites GigaVoxels. The trick behind trillion-voxel demos: the world is huge on disk or regenerated from a seed, but the GPU holds only the bricks rays touched recently, at the detail level each pixel needs.
- **Adopted: a ray-guided brick cache.**
  - The GPU keeps a fixed node pool and brick pool.
  - A ray that reaches a missing brick writes a request to a feedback buffer. The CPU streams those bricks in and evicts the least recently used.
  - Distant rays stop at coarse mip levels.
  - Result: GPU memory scales with screen resolution, not with world size.
- **Scale check at 1/64 m.** A trillion surface voxels at 4,096 per m² cover about 244 km², a map of about 15.6 × 15.6 km. Stored raw at about 1.1 bytes each, that's roughly 1.1 TB. So the base world is regenerated from the seed per chunk, and saves hold only player edits and destruction.
- **Single-player, first person.**
  - No networking or physics determinism, so Rapier runs in its fastest parallel mode.
  - The camera is always inside the world, so nearby detail matters most.
  - The player has a full cartoon-proportioned body, visible in shadows and reflections, with one character controller.
- **Your PC (dev machine).** i7-13620H with 10 cores and 16 threads, and an RTX 4070 Laptop GPU with 8 GB VRAM on DX12 FL 12.2. It can run the tier A extras: hardware RT and DLSS.
- **The Mac (the floor).**
  - Every 2019 16-inch MacBook Pro pairs Intel UHD 630 with a discrete AMD GPU. The [base model has a Radeon Pro 5300M with 4 GB GDDR6](https://appleinsider.com/articles/19/11/13/amd-details-the-16-inch-macbook-pros-radeon-pro-5000m-series-gpus), and [16 GB of RAM is onboard](https://everymac.com/systems/apple/macbook_pro/specs/macbook-pro-core-i7-2.6-six-core-16-2019-scissor-specs.html). Yours is likely 16 GB RAM and 4 GB VRAM, not 8; About This Mac will confirm.
  - Bevy asks wgpu for the [high-performance adapter by default](https://docs.rs/crate/bevy_render/0.15.1/source/src/settings.rs), so it picks the AMD GPU, not the Intel one.
  - No hardware RT and no DLSS, so it's tier B: pure compute. Budget the brick pool at 1 GB or less.
  - The [native panel is 3072×1920](https://www.bhphotovideo.com/c/product/1520434-REG/apple_mvvj2ll_a_16_macbook_pro_late.html), so render around 720p internally and upscale.
- **The Intel Mac's shelf life.** [macOS 26 Tahoe is the last release for Intel Macs](https://www.guru3d.com/story/apple-ends-major-macos-support-for-intel-macs-after-tahoe-26/), and this model is one of four Intel Macs it supports. Tahoe [gets security updates until fall 2028](https://www.thurrott.com/?p=321965). So we also build for Apple Silicon from day one. That arm64 build is also the presentation build: you'll demo on an M5 MacBook Air, while the 2019 MacBook Pro stays the performance floor and your PC stays the dev machine.
- **Testing without a Mac.** A `--mac-floor` mode on the PC forces tier B, caps the brick pool, renders at 720p internally and uses fewer worker threads. It can't catch Metal-specific bugs, so the MacBook Pro tests each macOS build (see Building for macOS).
- **Physics stays coarse.** Collision runs on a 1/16 m grid, 64× fewer voxels than the 1/64 m render grid. Players can't feel 1.6 cm steps, and a 2019 laptop CPU can't afford them.
- **Fixed world.** A map under about 16 km keeps f32 precise to about 2 mm, so we skip floating-origin and infinite-streaming code.

## What we're building

We're building a real-time 3D game where everything visible is made of centimetre-scale voxels. It runs rigid-body physics with destruction, on whatever graphics API the player's machine has.

Reference projects:

- **John Lin's micro-voxel demos** set the visual target: fine voxels with path-traced light.
- **[Douglas Dwyer's Octo](https://github.wsshow666.workers.dev/DouglasDwyer/octo-release)** is written in Rust and renders by ray marching voxels in compute shaders. It runs natively and on the web through WASM and WebGPU, and it targets integrated GPUs. It's the closest public proof that this stack works.
- **[Teardown](https://80.lv/articles/teardown-developer-breaks-down-multiplayer-and-voxel-destruction-tech)** models every object as a voxel volume on a regular grid, and the whole engine is built around that. It uses [dense per-shape voxel storage](https://blog.voxagon.se/2024/12/29/year-summary.html), which is the template for our dynamic objects.
- **MishMash (@MishMash95)**: only the channel's tagline loaded ("hobbyist GameDev interested in Computer Graphics"), not the video list. Tell me which videos to match.

What "no meshes" means in practice: the world, props and characters are all voxel data. Triangles remain only for UI, debug gizmos and the proxy boxes that start rays. Content comes from voxel editors, SDF graphs, procedural rules, or meshes converted to voxels offline.

## Bevy baseline and multi-backend rendering

We pin Bevy 0.20. wgpu already picks the best graphics API at runtime, so the real engineering is in capability tiers, not backend plumbing.

- Bevy ships a breaking release [about every 3 months](https://docs.rs/crate/bevy/0.18.1/source/README.md). The [0.20 release](https://bevy.org/news/bevy-0-20/) adds several things we use. WESL shaders bring modules, imports and conditional compilation. Mesh shaders arrive, though not on the web. Solari path tracing now runs on macOS through Metal. The release also adds schedule randomization and panic catching.
- Bevy renders through wgpu. By default `WgpuSettings.backends` enables DX12, Vulkan and Metal on native and [picks the best one automatically](https://docs.rs/crate/bevy_render/0.17.2/source/src/settings.rs). The `WGPU_BACKEND` env var overrides the choice ([vulkan, dx12, metal, gl](https://docs.rs/crate/bevy-sensor/0.4.3/source/WEBGPU_RESEARCH.md)).
- What we ship: one build per OS with every primary backend compiled in, auto-selection, and a `--backend` CLI flag plus a settings entry. A backend change applies on restart, because the GPU device is created at startup.

Running on any backend doesn't mean the same features everywhere. At startup we read the adapter's features and limits, pick a tier, and register render plugins to match.

| Tier | Backends | Needs | What runs |
| --- | --- | --- | --- |
| A: High | Vulkan, DX12 or Metal on ray-tracing GPUs ([RTX 2000+, RX 6000+](https://github.meowingcats01.workers.dev/bevyengine/bevy/pull/10000)) | Compute, storage buffers, ray queries, mesh shaders | Full voxel renderer, plus hardware-RT extras such as Solari for any triangle content |
| B: Baseline | Vulkan, DX12, Metal, browser WebGPU | Compute shaders, storage buffers | Full voxel renderer through compute traversal, with software GI |
| C: Unsupported | GL, WebGL2 | No compute shaders | The world renderer can't run here, so we show a clear error |

Open point: I haven't confirmed whether wgpu ray queries accept custom AABB geometry, which voxels would need. If they only accept triangles, hardware RT can't accelerate voxel traversal. Either way, compute traversal is the baseline on every tier.

## Voxel scale and memory budget

Chosen voxel size: 1/64 m (1.56 cm). At that size a 1.8 m character is about 115 voxels tall, against 2 in Minecraft and about 18 at Teardown's \~10 cm. The LOD-ring math below assumed 1/32 m. The ray-guided brick cache under What your answers changed replaces it as the memory plan.

| Voxel size | Voxels per m³ | 1.8 m character height (voxels) | Surface voxels per m² |
| --- | --- | --- | --- |
| 1/16 m (6.25 cm) | 4,096 | 29 | 256 |
| **1/32 m (3.125 cm)** | **32,768** | **58** | **1,024** |
| 1/64 m (1.56 cm) | 262,144 | 115 | 4,096 |
| 1 cm | 1,000,000 | 180 | 10,000 |

Dense storage is impossible at this size. A region 1 km × 1 km × 256 m at 1/32 m holds 32,000 × 32,000 × 8,192 ≈ 8.4 × 10¹² voxels. Only surfaces cost memory, because uniform interiors collapse into single tree nodes.

LOD budget (estimate, superseded by the brick cache). Full resolution covers a 64 m radius. Each ring outward doubles the radius and halves the resolution, so every ring holds about the same number of surface voxels:

```latex
N_{ring} \approx 3\pi r^2 \cdot \tfrac{1024}{4} = 3\pi (64)^2 (256) \approx 9.9 \times 10^6
```

- The full-resolution disc holds about 13.2 M voxels. Six rings reach about 4 km and hold about 72 M surface voxels in total.
- Doubling for surface thickness gives about 140 M voxels. At about 1.1 bytes each (occupancy bits plus a 1-byte palette index), that's **about 160 MB of GPU memory**. It fits an 8 GB card with room for lighting caches.
- At 1/64 m the same layout is about 4× larger, roughly 640 MB, unless the full-resolution radius drops to 32 m.

Different object classes can use different resolutions. Dynamic objects own their own grids, so a hand-held item or a character's face can be 1/64 m while terrain stays 1/32 m.

Precision: at 10 km from the origin, f32 resolves positions to about 1 mm (2¹³–2¹⁴ m range → 2⁻¹⁰ m steps). That's fine up to tens of kilometres. A larger world needs a floating origin, for example the big\_space crate.

## Voxel data structures

The static world goes in a sparse 64-tree: each node has 4×4×4 children and a 64-bit child mask. Dynamic objects get small dense grids.

| Rank | Structure | Edit cost | Ray traversal | Memory | Verdict |
| --- | --- | --- | --- | --- | --- |
| 1 | Sparse 64-tree ([guide](https://dubiousconst282.github.io/2024/10/03/voxel-ray-tracing)) | O(log n) by path copying | Fast: half an octree's depth, skips empty space using bitmasks | Bits per voxel, plus a palette | **Winner for the world** |
| 2 | Brickmap (flat grid of 8³ bricks) | O(1) | Fast | Wasteful for large or empty worlds | Runner-up, and the fastest to prototype |
| 3 | SVO (8-ary octree) | O(log n) | Twice as deep, so more stack steps | Good | Lost: the 64-tree beats it |
| 4 | SVDAG (deduplicated tree) | High: edits need re-deduplication | Fast | Best compression | Lost: destruction edits constantly |
| 5 | Dense chunks (Teardown-style) | O(1) | Simple DDA | \~1 byte per voxel, too big for a cm-scale world | **Use for dynamic objects only** |

Why the runner-up might still be the right start: a brickmap is far simpler to write, and it's enough for a bounded map of roughly 512 m or less. If you choose a small arena world, prototype on a brickmap first.

Existing crates, and why we write our own core:

- [VoxelHex](https://relatedrepos.com/gh/Ministry-of-Voxel-Affairs/VoxelHex) is a sparse voxel-brick tree with ray tracing. Its last update was Oct 2025, so it predates Bevy 0.20 and would need porting. Use it as a reference, not a dependency.
- [voxelis](https://awesome.ecosyste.ms/projects/github.com%2Fwildpixelgames%2Fvoxelis) is a pure-Rust SVO-DAG with chunk grids, tagged for Bevy. It's good for static compression and weaker for constant edits.
- [tree64](https://relatedrepos.com/gh/expenses/tree64) is a Rust 64-tree with hashing and modifications, built from the guide above. It's the closest starting point.
- Recommendation: own `world_core`, since it's the heart of the game, and borrow ideas from tree64 and the guide.

Per-voxel data:

- Each voxel stores a u16 material id, indexed through a per-node palette.
- A material table holds color, roughness, emission, density, strength and friction.
- Normals are computed at render time from neighboring voxels instead of being stored.

## Physics engine choice

Chosen: **Rapier**, through bevy\_rapier3d 0.37 on rapier3d 0.36.1, released Oct 10, 2026 for Bevy 0.20. It wins because it ships a native voxel collider. Avian 0.8 is the runner-up, and it wins if you want to own the collision code.

| Rank | Engine | Bevy 0.20 | Voxel colliders | Customizability | Verdict |
| --- | --- | --- | --- | --- | --- |
| 1 | [Rapier](https://docs.rs/crate/bevy_rapier3d/latest) | Yes, 0.37 (Oct 10, 2026) | [Built-in `Voxels` shape](https://rapier.rs/docs/user_guides/bevy_plugin/colliders). [Sparse chunk storage, dynamic voxel bodies, and voxel-to-voxel contact forces](https://classic.yarnpkg.com/en/package/@dimforge/rapier3d). | Medium. It has hooks and a custom broad-phase trait, but runs a separate physics world mirrored into the ECS. | **Winner** |
| 2 | [Avian](https://docs.rs/crate/avian3d/latest) | Yes, 0.8 (Oct 20, 2026) | Parry shapes: [Collider wraps SharedShape](https://idanarye.github.io/bevy-tnua/avian3d/collision/collider/struct.Collider.html). Whether Avian's pipeline handles voxel contacts is unverified. A [custom collider trait](https://docs.rs/crate/avian3d/0.6.1/source/src/collision/collider/mod.rs) lets us write our own. | Highest. ECS-native, modular plugins, custom collision backends, f64. | Runner-up |
| 3 | Jolt ([rolt](https://docs.rs/crate/rolt/latest), [oxijolt-sys](https://docs.rs/crate/oxijolt-sys)) | No Bevy plugin | Not evaluated | rolt targets Jolt 5.0 and is "best-effort and incomplete". oxijolt-sys binds 5.6, unsafe only. | Lost: C++ build (CMake, libclang), no Bevy integration, large unsafe surface |
| 4 | PhysX (physx-rs) | No | No | Bindings stale | Lost |
| 5 | Custom voxel solver | n/a | Whatever we write | Total | Lost for now: months of work. Teardown's author [rebuilt his physics engine](https://blog.voxagon.se/2024/12/29/year-summary.html) to keep improving it. |

The evidence so far is weak:

- A Godot-hosted benchmark found [Rapier fastest on three of five scenes against Jolt 5.5 and Box3D, and 4× faster on queries](https://forum.godotengine.org/t/physics-engine-comparison-rapier-vs-godot-vs-box2d-3d-vs-jolt/142786). It's third-party and not our workload.
- Avian 0.6's BVH broad phase was [10× faster on a scene with 40,000 static colliders](https://hn.jakechampion.name/reader/https://joonaa.dev/blog/12/avian-0-6).
- No test exists for micro-voxel workloads, hence the spike.

The case for Avian: if you want the physics to read like our own code, as systems you can reorder and replace in Bevy's schedule, Avian is the better long-term home. Its custom-collider trait is the clean route to a micro-voxel narrow phase.

Validation (about 1 week). Build this scene in Rapier: a 64 × 64 m voxel terrain collider, 200 dynamic debris pieces of 2k–20k voxels each, and one character controller. Measure ms per step on your machine, stability (jitter, tunneling) and island-split cost. Cap the run at 4 worker threads to approximate a base Mac. If it can't hold 60 Hz there, the fallback is a coarser collision grid first, then Avian.

## Voxel physics design

Neither Rapier nor Avian does fracture, soft bodies or fluids. Realism comes from our own voxel layer on top: destruction, materials and support.

- **Two resolutions.** Render voxels are 1/64 m and collision voxels are 1/16 m: 64× fewer voxels to collide. The collision grid size stays a tuning knob.
- **Static world.** Voxel colliders come in chunks (for example 4 m), built only near awake bodies and the player, and rebuilt when edited.
- **Dynamic objects.** Each is a rigid body with a voxel collider over its own grid. Mass and inertia are summed from each voxel's material density.
- **Destruction.** Damage removes or weakens voxels. A union-find or flood fill then runs over the edited object, and each disconnected island becomes a new body. Terrain islands with no path to an anchor such as bedrock fall. The work is time-sliced to N voxels per frame.
- **Debris budget.** These numbers are starting guesses, tuned once destruction runs. Islands under about 64 voxels become GPU particles, not rigid bodies. Sleeping is on, and active bodies are hard-capped at about 500.
- **Character controller.** A kinematic capsule with move-and-slide, about 0.3 m step-up and ground snapping, because fine voxels put tiny stair-steps everywhere. Rapier provides `KinematicCharacterController`, and Avian 0.6+ provides [MoveAndSlide](https://hn.jakechampion.name/reader/https://joonaa.dev/blog/12/avian-0-6).
- **Timestep.** Physics runs at a fixed 60 Hz in `FixedUpdate` with transform interpolation. Single-player, so Rapier runs in parallel mode without enhanced determinism.
- **Materials.** One table drives friction, restitution, density, strength and break sounds.
- **Later, pending your scope answer:** a structural stress graph, plus fire, sand and fluid cellular automata.

## Animation engine

Each bone owns a rigid voxel part. Bevy's animation system drives the parts, with a procedural IK layer and a physics ragdoll on top.

The player has a full body even in first person, because it shows in shadows and reflections. Its proportions are cartoon-style, with a bigger head and hands, which suits rigid voxel parts. The local player's custom rig comes first. Creatures, NPCs and animals come later and reuse the same rig system.

| Rank | Approach | Strength | Cost | Verdict |
| --- | --- | --- | --- | --- |
| 1 | Rigid voxel part per bone | Cheap. Maps one-to-one onto ragdoll bodies and joints. Limbs can break off because they're already separate objects. Bevy's AnimationGraph animates the bone Transforms like any entity hierarchy. | Seams at joints, hidden by overlapping parts and joint-cap voxels | **Pick** |
| 2 | Shell-mesh ray deformation | Smooth bending. An animated shell is rasterized, and each ray is mapped back into the bind-pose voxel model, which never changes ([reference implementation, 2048³ SVO](https://www.github.com/nieznanysprawiciel/VoxelRenderer)). | Complex. Interacts badly with destruction and physics. | Later, for faces and creatures |
| 3 | Re-voxelize a skinned mesh every frame | Animate in Blender with standard rigs | Per-frame cost grows with resolution, and it breaks the no-mesh art direction | Lost |
| 4 | Per-frame voxel flipbooks | Simple | Huge memory use | Lost |

Procedural layer:

- Two-bone IK plants feet on uneven voxel ground.
- Look-at, procedural walk cycles for creatures, and spring-driven secondary motion.
- Vegetation sways through ray-space warping in the shader, so no voxel data moves.

Authoring: import parts from MagicaVoxel .vox files (crate version to verify against Bevy 0.20). An in-game part editor comes later.

## Procedural generation engine

World generation is a deterministic pipeline addressed by chunk: the same seed and chunk coordinate produce the same voxels on every machine.

1. **Macro.** Continents, a heightmap and biomes from noise (fBm, domain warping). An optional hydraulic-erosion pass runs on a coarse heightmap.
2. **Meso.** Caves (3D noise, worm tunnels), rivers and cliffs, all expressed as SDFs.
3. **Props.** Rocks, trees and ruins are SDF graphs and grammars (L-systems, space colonization for trees). They're voxelized at whatever resolution the current LOD needs, so props are resolution-independent and LOD comes for free.
4. **Surface detail.** Material layers (soil, moss, rock strata) are chosen by noise over depth and slope.
5. **Edits.** Player and destruction changes are stored as an overlay on the generated base, which keeps save files small.

Determinism rule: floating-point noise on GPUs can differ between vendors. Authoritative generation therefore runs on the CPU (rayon-parallel). Multiplayer is out, but a save moved between your PC and the Mac must regenerate the same base terrain, or edits would float. So GPU generation must use integer math, or stay visual-only.

Worldgen parameters live in hot-reloadable RON assets. Candidate noise crates are noise and fastnoise-lite, with versions still to verify.

## Hot reloading and iteration speed

Iteration speed comes from five layers, cheapest first. Most tuning should never need a recompile.

1. **Assets.** Bevy's `file_watcher` feature [hot-reloads assets on file change](https://vleue.github.io/bevy_workshop-eurorust-2025/0-intro/hot-patching.html): shaders, textures and configs.
2. **Data-driven tuning.** Materials, physics constants and worldgen parameters live in RON assets, with a bevy-inspector-egui panel for live edits.
3. **Rust hotpatching.** Since Bevy 0.17, `dx serve --hot-patch --features "bevy/hotpatching"` [patches systems in a running app](https://vleue.github.io/bevy_workshop-eurorust-2025/0-intro/hot-patching.html), using Dioxus's [subsecond](https://docs.rs/subsecond/=0.7.10/), which routes function calls through a jump table. Limits:
   - It covers systems in the `Update` schedule.
   - [Struct layout changes can crash after a patch](https://www.mintlify.com/DioxusLabs/dioxus/cli/hot-reload).
   - Workspaces split into many crates can be slow to patch or fail to patch.
4. **Fast rebuilds for everything else.** `bevy/dynamic_linking` in dev builds, the lld or mold linker, opt-level 1 for our crates and 3 for dependencies, and optionally the Cranelift backend.
5. **Scripting (optional).** Lua through bevy\_mod\_scripting for gameplay and mod logic, only if mods are in scope.

Tension: hotpatching works best in few crates, while testability wants many. Resolution: keep fast-changing gameplay systems in `world_game` and `app`, and keep stable engine code in separate crates.

## Testing and CI

Each layer is tested against a simpler reference implementation, so the GPU, physics and worldgen code always has something trustworthy to be compared with.

1. **Core crate (no Bevy).** Unit tests and property tests (proptest): random edits checked against a dense reference array, serialization round-trips, and tree traversal checked against a brute-force DDA ray reference.
2. **ECS integration.** A headless App with MinimalPlugins steps `app.update()` N frames, then asserts on world state.
3. **Physics.** Deterministic replay: run a scenario twice, hash body transforms every step, and require the hashes to match. Regression scenes, such as a tower collapse, carry recorded hashes.
4. **Rendering.**
   - Headless renders compared against golden images within a tolerance, on CI with software Vulkan (lavapipe) and DX12 WARP.
   - On small scenes, GPU ray hits must match a CPU reference renderer exactly.
5. **Performance.** Criterion benchmarks for tree edits and traversal, GPU timestamp queries, and Tracy through Bevy's tracing features. CI fails on a frame-time regression above a set threshold.
6. **Ordering bugs.** Bevy 0.20 adds [schedule randomization](https://bevy.org/news/bevy-0-20/), which we turn on in tests to flush out hidden system-ordering assumptions. The exact API is still to check.
7. **Worldgen.** A golden hash per seed and chunk catches nondeterminism.

## Workspace layout and adaptability

The repo becomes a Cargo workspace. The voxel core and worldgen have zero Bevy dependencies, so they test fast, survive Bevy upgrades unchanged and can run on a server.

```
World/
  crates/
    world_core/     voxel types, 64-tree, palette, edits, serialization (no Bevy)
    world_gen/      procedural pipeline (depends on world_core only)
    world_render/   Bevy render plugin, WESL shaders, capability tiers
    world_physics/  world_core <-> Rapier/Avian bridge, destruction, islands
    world_anim/     voxel rigs, IK, ragdoll mapping
    world_game/     gameplay systems (hot-patched most)
  app/              binary: backend selection, settings, CLI
  tools/            xtask: .vox and mesh-to-voxel baking, benchmarks
  assets/           shaders/, materials.ron, worldgen.ron
```

Rules:

- The physics engine is confined to `world_physics`, so swapping Rapier for Avian touches one crate.
- Render tiers are Cargo features plus runtime detection.
- Every crate boundary is a Bevy plugin with a single public entry point.

## Building for macOS

No decompilers or emulators are involved. Rust compiles the same source code straight to native machine code for each platform, one target per OS and CPU.

| Target | Runs on | Built on |
| --- | --- | --- |
| `x86_64-pc-windows-msvc` | Your PC | Your PC |
| `x86_64-apple-darwin` | The 2019 MacBook Pro (Intel) | Any Mac, or GitHub's `macos-15-intel` runner |
| `aarch64-apple-darwin` | Apple Silicon Macs (M1 and later) | Any Mac, or GitHub's arm64 macOS runners |

1. **Compile.** A Mac's toolchain builds both Mac targets: `cargo build --release --target x86_64-apple-darwin`, then the same with `aarch64-apple-darwin`. Without a Mac, GitHub Actions macOS runners do it on every push. Cross-compiling from Windows needs Apple's macOS SDK, which is licensed for Apple hardware, so we skip that route.
2. **Merge.** `lipo` combines the two builds into one universal binary that runs natively on Intel and Apple Silicon.
3. **Bundle.** cargo-bundle or cargo-packager wraps the binary and the `assets/` folder into a `.app`, then a `.dmg`.
4. **Sign.** An ad-hoc signature is enough for your own Mac, though the first launch needs right-click → Open. Sharing it publicly needs an Apple Developer ID and notarization, or macOS blocks it.

Deadline: GitHub's [macos-15-intel runner is the last Intel runner and is supported until August 2027](https://core.trac.wordpress.org/ticket/64246). The Intel build stays a target until then. The arm64 build is what keeps the game running on newer Macs afterward.

## To-do

These tasks are in build order, with no phases and no dates. Each one says what it involves and when it counts as done.

- [ ] **Set up the Cargo workspace** Create the crates from Workspace layout. Pin bevy 0.20 and bevy\_rapier3d 0.37. Dev profile: `bevy/dynamic_linking`, opt-level 1 for our crates and 3 for dependencies. Done when `cargo run` opens an empty Bevy window on your PC.
- [ ] **Detect backend and tier** Read the adapter's features and limits at startup, then log the backend and tier. Add a `--backend` flag and a `--mac-floor` mode (tier B, 720p internal, capped brick pool, 4 worker threads). Done when DX12, Vulkan and mac-floor all boot.
- [ ] **Set up CI** Windows build and tests, render tests on software Vulkan (lavapipe), and macOS builds for x86\_64 and arm64 (see Building for macOS). Done when every push goes green.
- [ ] **Build the voxel core** 64-tree, palette, edits and serialization in `world_core`, with no Bevy dependency. Done when property tests pass: 1M random edits with no divergence from a dense reference array.
- [ ] **Write a CPU reference ray tracer** Slow but exact. Done when it renders a test scene to a PNG. It's the ground truth for the GPU tracer.
- [ ] **Build the GPU primary-ray pass** Compute traversal into the G-buffer, writing depth into Bevy's depth texture. Done when GPU hits match the CPU reference exactly on test scenes.
- [ ] **Build the ray-guided brick cache** Feedback buffer, CPU streaming, LRU eviction, fixed pool size. Done when a 2 km walk stays inside the pool with no frame over 16 ms in mac-floor mode.
- [ ] **Procedural terrain v1** Seed plus chunk coordinate gives voxels, identical on Windows and Mac, with an edit overlay in saves. Done when the same seed hashes the same on both machines.
- [ ] **First-person controller** Rapier kinematic capsule with move-and-slide, about 0.3 m step-up, ground snapping, and a 1/16 m collision grid. Done when you can walk over rough voxel ground without catching or jittering.
- [ ] **Local player with a custom rig** A full-body, cartoon-proportioned skeleton (bigger head and hands), defined in a RON asset (bone names, lengths, joint limits), with one voxel part per bone. True first person: the camera sits at the eyes, the head is hidden from the main view only, and the full body shows in shadows and reflections. Animate it with AnimationGraph, plus foot IK and procedural sway and recoil. The rig hot-reloads when the RON file changes. Done when the player walks rough ground and holds and swings a tool, and a rig edit shows up without a recompile.
- [ ] **Rapier validation scene** 64 × 64 m voxel terrain, 200 debris pieces, capped at 4 threads. Done when it holds 60 Hz physics. If it can't, coarsen the collision grid first, then try Avian.
- [ ] **Lighting** Sun shadows and AO first, then the GI cache, temporal accumulation and upscaling. Done when GI is stable with no visible boiling at 60 fps on your PC and 30 fps or more in mac-floor mode.
- [ ] **Reflections** Secondary rays through the voxel tree, plus a GPU BVH over dynamic-object boxes so the player and props appear in reflections. Roughness controls sharpness. Half resolution in mac-floor mode. Done when the full player body shows correctly in water and on a glossy floor, at 30 fps or more in mac-floor mode.
- [ ] **Dynamic objects and destruction** Box-march pass, island splitting, debris particles, active-body cap. Done when a wall breaks into falling pieces without a physics stall.
- [ ] **Creatures, NPCs and animals (later)** Reuse the player's rig system: rigid-part rigs, foot IK, ragdoll on hit, plus behavior and AI.
- [ ] **Package for macOS and test on both Macs** Universal `.app`, ad-hoc signed. Done when it launches on the 2019 MacBook Pro and the M5 MacBook Air, and hits each machine's targets in How the game looks and renders.
- [ ] **Gameplay prototype** The core loop you pick. Done when there's a playable 10-minute session.

## Risks and corrections to assumptions

The biggest risk is building an engine with no game decided. Several of the original assumptions also need correcting.

Corrections:

- **"Run on whatever backend the computer has"** is already wgpu's default. The real work is feature tiers, and WebGL2 can't run the voxel renderer at all.
- **"The best physics engine"** doesn't exist for voxels. Rapier and Avian are rigid-body solvers. Destruction, materials and stress are our own layer on top of either one.
- **"As realistic as possible" with very fine voxels** pull against each other. Each halving of voxel size costs about 4× in surface data and 8× in volume. Pick the resolution per object class, not globally.
- **"No meshes"** still leaves triangles for UI, debug drawing and the proxy boxes that start rays.
- **VoxelHex as a foundation**: last updated Oct 2025, so it trails Bevy 0.20. Use it as a reference, not a dependency.

| Risk | Impact | Mitigation |
| --- | --- | --- |
| Scope: an engine with no game | Never ships | Decide the game type now. The gameplay prototype is on the to-do list. Polish waits until it's playable. |
| GPU memory and bandwidth at fine scale | Frame drops | Ray-guided brick cache with a fixed pool, a coarser collision grid, measured once the cache runs |
| Physics body explosion from destruction | Physics stalls | Debris becomes particles, sleeping, an active-body cap |
| Bevy breaking release every \~3 months | Upgrade cost | Pin versions. One upgrade window per release. Core crates stay Bevy-free. |
| Multiplayer added late | Rewrite of physics and worldgen | Ruled out: single-player only |
| Hotpatch limits | Slower iteration | Data-driven tuning first. Hot systems stay in a few crates. |

## Open questions for Tony

The answers set the physics engine, the voxel size and the hardware floor. Setup tasks can start before they're all in.

Answered Nov 18: physics (Rapier), voxel size (1/64 m), hardware (Windows RTX 4070 Laptop for development, a 2019 16-inch MacBook Pro as the floor), world (fixed, large), single-player first person, painterly art, a macOS build for your own use. Still open: questions 9–13, plus these:

- Mac specs: deferred until you have the MacBook Pro. Check About This Mac → Graphics, Memory then.
- Body view: answered. Full body in first person, with full reflections.
- Rig proportions: answered. Cartoon-style, good-enough proportions.
- Still open: what the reflections should look like (mirror-sharp, glossy, or painterly blur).

1. **Physics:** Rapier (built-in voxel colliders, mature) or Avian (ECS-native, we own the collision code)? Or a custom solver later?
2. **Voxel size:** 1/16 m, 1/32 m (recommended), 1/64 m, or 1 cm?
3. **Hardware floor:** ray-tracing GPUs only; any DX12, Vulkan or Metal GPU including integrated; or browsers through WebGPU as well?
4. **Game type:** a destruction sandbox (Teardown-like), open-world survival or exploration, or something else?
5. **Multiplayer:** never, co-op later, or competitive? This decides the determinism requirements.
6. **World size:** a bounded map (1 km or less), large (up to 16 km), or infinite?
7. **Art direction:** photoreal PBR, stylized, or painterly like John Lin's demos?
8. **Your machine:** GPU model, CPU and RAM. These set the performance targets.
9. **Time:** hours per week and team size. These set the pace of the to-do list.
10. **Platforms for v1:** Windows only, or Windows, Linux and macOS?
11. **Mods:** do you want Lua scripting or mod support?
12. **References:** which MishMash videos should I match?
13. **Simulation scope:** fluids, fire, sand, or structural stress beyond rigid bodies?

## Sources

- [Bevy 0.20 release notes](https://bevy.org/news/bevy-0-20/) · [Bevy 0.19 release notes](https://bevy.org/news/bevy-0-19/)
- [bevy\_render WgpuSettings (backends)](https://docs.rs/crate/bevy_render/0.17.2/source/src/settings.rs) · [wgpu Backends](https://doc.qu1x.dev/bevy_trackball/bevy/render/settings/struct.Backends.html)
- [Bevy hot-patching workshop (EuroRust 2025)](https://vleue.github.io/bevy_workshop-eurorust-2025/0-intro/hot-patching.html) · [subsecond docs](https://docs.rs/subsecond/=0.7.10/) · [Bevy hotpatching upstreaming plan](https://hackmd.io/@bevy/SkdMQIYWle)
- [avian3d versions (0.8.0 for Bevy 0.20)](https://docs.rs/crate/avian3d/latest) · [Avian 0.6 release post](https://hn.jakechampion.name/reader/https://joonaa.dev/blog/12/avian-0-6)
- [bevy\_rapier3d versions (0.37.0 for Bevy 0.20)](https://docs.rs/crate/bevy_rapier3d/latest) · [Rapier voxel colliders](https://rapier.rs/docs/user_guides/bevy_plugin/colliders) · [Rapier JS changelog (voxel features)](https://classic.yarnpkg.com/en/package/@dimforge/rapier3d)
- [Rapier vs Jolt vs Box3D benchmark (Godot)](https://forum.godotengine.org/t/physics-engine-comparison-rapier-vs-godot-vs-box2d-3d-vs-jolt/142786)
- [rolt / Jolt Rust bindings](https://docs.rs/crate/rolt/latest) · [oxijolt-sys](https://docs.rs/crate/oxijolt-sys)
- [Sparse 64-tree voxel ray tracing guide](https://dubiousconst282.github.io/2024/10/03/voxel-ray-tracing) · [VoxelRT experiments](https://backiee.wasmer.app/http_github_com/dubiousconst282/VoxelRT)
- [VoxelHex](https://relatedrepos.com/gh/Ministry-of-Voxel-Affairs/VoxelHex) · [tree64](https://relatedrepos.com/gh/expenses/tree64) · [voxelis](https://awesome.ecosyste.ms/projects/github.com%2Fwildpixelgames%2Fvoxelis)
- [Octo engine (Douglas Dwyer)](https://github.wsshow666.workers.dev/DouglasDwyer/octo-release)
- [Teardown voxel tech interview](https://80.lv/articles/teardown-developer-breaks-down-multiplayer-and-voxel-destruction-tech) · [Voxagon 2024 summary](https://blog.voxagon.se/2024/12/29/year-summary.html) · [Voxel physics write-up (Milan Bonten)](https://milanbonten.github.io/voxel-physics)
- [Voxel skeletal animation via shell meshes](https://www.github.com/nieznanysprawiciel/VoxelRenderer)
- [MishMash channel](https://www.youtube.com/@MishMash95)
