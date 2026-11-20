# 2026-11-20: Floor Mac setup, checks and performance

The first working session on the floor Mac: a 2019 16-inch MacBook Pro,
i7-9750H (6 cores, 12 threads), 16 GB, AMD Radeon Pro 5300M (4 GB) and Intel
UHD 630, macOS 26.7. It answers the checklist in
`2026-11-19-macbook-first-test.md` and records what the first measurements
found. All numbers are development builds (`cargo run`, opt-level 1 for our
crates) unless marked.

## Setup

Nothing was installed. The Xcode Command Line Tools (linker, SDK, git) come
first; everything else installs without Homebrew: rustup (1.97.1 from
`rust-toolchain.toml`, nightly with Miri), git-lfs from its release zip, and
cargo-nextest, cargo-llvm-cov, cargo-fuzz and cargo-mutants as prebuilt
binaries through cargo-binstall. dx must be built from source
(`cargo install dioxus-cli --version 0.7.10 --locked`): the prebuilt binary
links Homebrew's OpenSSL and aborts at launch without it. A clean build takes
26 minutes; compiling the test targets adds about 11.

## Checklist results

- **Adapters:** both GPUs on Metal, tier B (`ray_query=false`,
  `mesh_shader=true`). The game picks the Radeon.
- **Tests:** `cargo xtask test all` with `WORLD_TEST_ADAPTER=gpu
  WORLD_TEST_REQUIRE_GPU=1`: 607 passed, 0 skipped. fmt, clippy (workspace,
  hotpatch and release feature sets) and the World_Gen golden hashes pass.
- **Golden images:** `cargo xtask golden` fails here by design: it requires a
  GPU and defaults to the software adapter, which Metal does not have. With
  `WORLD_TEST_ADAPTER=gpu` every golden image matches on the Radeon.
- **Benchmarks:** random reads 3.13 ms per 100k (Windows reference 2.3), a
  chunk with caves 133 to 143 ms (104 to 112), about 1.3 times the Windows
  laptop, as expected from the CPU.

## What the floor Mac found

In order of discovery:

1. **`internal_resolution` was never applied, and the RON pool sizes were
   ignored** (fixed in 79bad9f, before this entry): a grey slab at the
   horizon and 4 fps in mac-floor.
2. **Zeroing workgroup memory cost 60% of the trace on Metal.** The trace's
   2.5 KiB ancestor stack was zeroed for every workgroup: 21.7 ms against
   8.3 ms at 1280x720 in `gpu_timing`. A private-array stack measured
   slower (10.0 ms; RDNA spills it).
3. **The debug scene kept editing after the real world replaced it**, 192
   dirty bricks every 4 frames, each re-running stage 1's full growth walk
   (52 ms).
4. **Stage 1's growth walk re-ran whole on any change** and the shrink walk
   never stopped (decision 0010).
5. **Summaries counted swapped-in chunks voxel by voxel**, about 6 million
   reads per surface chunk with caves, 60 ms a frame while walking.
6. **Stale queues escaped the per-frame budget** while walking (decision
   0010).

Walking at 4 m/s in mac-floor, CPU time in the brick cache's `run_frame` per
frame: mean 86.0 to 11.9 ms, p95 352.7 to 23.2 ms, max 942 to 77 ms.

## Measuring on this Mac

- **Window focus matters.** Bevy's default lets an unfocused window wait on
  a timer, which App Nap stretches to 20-second gaps when the game window is
  behind other windows. Runs with `--exit-after-frames` now use continuous
  updates.
- **The display must be awake and unlocked.** With the display asleep or the
  screen locked the window gets no drawable: screenshots come out black and
  frames skip the GPU work, so whole-frame times look far better than they
  are. Every whole-frame number in this entry was taken with the display on,
  before it slept at 22:47; the walk measurements are the render world's own
  `run_frame` times, which do not depend on the display. `ioreg -n Root -d1 |
  grep CGSSessionScreenIsLocked` tells.
- **Other builds on the machine double the noise.** Six cores do not leave
  room for a test suite and a measurement at once.
- Tools added for this: `--frame-times PATH` (every frame's time),
  `--walk M_PER_S` (an autopilot over the terrain), and the frame-time
  summary at the end of fixed-length runs.

Whole-frame times with the display on, still camera, 600 frames, frames
101 to 600: mac-floor 59.9 ms mean at the start of the session; default
profile 130.4 ms. They need measuring again with every change above, the
display awake and the screen unlocked, still and walking.

## Hot-patching on macOS

`cargo xtask hot --smoke -- --exit-after-frames 100000` passes on this Mac,
the first run outside Windows: the first `dx serve --hot-patch` build took
19.5 minutes (Bevy without dynamic linking), and an edit to
`app/src/startup/hot_probe.rs` reached the running game in 3.1 s (dx
reported 2.8 s for the patch). dx was built from source against a static
OpenSSL 3.5 (`OPENSSL_DIR=$HOME/.local/openssl OPENSSL_STATIC=1 cargo install
dioxus-cli --version 0.7.10 --locked`), since there is no Homebrew here.

## Warm cache and prefetch

Decision 0006's warm tier now also holds unedited chunks evicted from the
hot tier (LZ4, least recently used first, edited chunks aging four times
slower), and the residency loads along the player's velocity: the chunks
ahead first, then a lookahead sphere around the predicted position.
Compression runs only when loads leave workers idle.

On this Mac residency is bound by generation, not order: about 6 to 8
chunks a second with the renderer competing, 57 to 290 ms per chunk, so
even at 4 m/s the full-detail sphere is about half empty while walking.
Prefetching moved the holes ahead of the player (12 m/s over 1500 frames:
185 to 157 holes per frame, 117 to 90 ahead) but did not remove them. A
chunk back from the warm cache takes 23 to 120 ms instead (an all-air
chunk 14 µs against 37 ms generated); the straight-line autopilot never
returns, so the walks show no warm hits. Surface chunks before buried ones,
or more workers, are the next things to try.

An independent review found two problems, fixed before merging: chunks
waiting to be compressed kept the hot total over its cap, so eviction took
another batch every frame; and a region file that failed to read was
treated as holding no edits, then kept warm, so a later save could
overwrite them. 622 tests pass on the Radeon, golden images match.

## Shared column footprints and surface-first loading

Every generated chunk computed its footprint's whole 1024 x 1024
heightfield, again for each chunk stacked in the same column, and an
all-air chunk cost as much as the heightfield. The workers now share a
bounded cache of footprints (`column_cache_mib`, 256 MiB, least recently
used out), a chunk whose footprint bounds prove it air or bedrock is not
built, and chunks entirely below their footprint's lowest column load as if
`buried_delay_m` (32 m) farther away.

Single-threaded, development build, five chunks of one column from two
below the surface to two above: an air chunk 36 ms to 5 µs, a stacked
built chunk 27 to 37 ms less (surface 72 to 45 ms, buried 261 to 224 ms),
the column 21 to 26% less. The first chunk of a column costs the same, and
buried chunks (cave carving, 110 to 270 ms) remain the expensive ones.

Walking at 4 m/s in mac-floor, 1500 frames, three runs each, screen
locked: holes 90 to 98 per frame on average before, 63 to 73 after; the
second five seconds 111 to 144 before, 72 to 96 after. Worker time per
generated chunk 176 to 213 ms before, 157 to 177 ms after, with 37% of the
loads uniform. At 8 m/s both versions fall behind the walk (holes near the
sphere's 280) after the first 40 s and the runs are too noisy to compare.
What remains is per-stacked-chunk work World_Gen's ChunkSource does again
(copying the columns and rebuilding their pyramid, about 10 ms) and the
cave grid and carving of buried chunks.

## Faster generation, and App Nap

World_Gen now builds chunks in about half the time with identical output
(`terrain_tree` benches: a surface chunk with caves 130 to 62 ms, an
underground one 122 to 58 ms, columns 24.6 to 15.6 ms; decision 0011).

Walks with the screen locked first showed the merged build collapsing after
about 30 s: frames of 150 to 800 ms and chunk generation at 0.4 to 1.2 s.
The cause was not the engine. The game's CPU use fell from about 510% to
70 to 140% at the same moment, with no thermal limit and free memory: App
Nap had lowered the priority of the hidden process. Continuous updates do
not prevent that. Fixed-length runs now hold an `NSProcessInfo` activity
(`app/src/startup/awake.rs`), and the collapse is gone. Earlier walk numbers
in this entry may include App Nap stretches; the residency hole counts are
the least affected, the slowest frames the most.

mac-floor, 1500 frames, both builds with the App Nap fix, alternating runs:

| | Before the merges | After |
|---|---|---|
| Sphere first settled | 17 to 21 s | 7.1 s |
| Holes at 4 m/s after 10 s | 33 to 62, then falling | 0 |
| Holes at 8 m/s after 10 s | 97 to 108, never caught up | 0 to 1 |
| Steady frames, 4 m/s | 16.8 to 17.9 ms mean, p95 28 | 20.6 to 21.5 ms, p95 32 to 36 |
| Steady frames, 8 m/s | 19.4 to 19.7 ms, max 105 | 23.8 to 24.2 ms, max 247 |

The renderer now streams a complete world instead of a half-empty one, which
costs about 4 ms a frame. The slowest frames come from the brick cache, whose
per-frame budget counts upload bytes but not CPU: a frame can expand 800 to
2,500 nodes (25 to 90 ms), an edit costs about 7 ms per dirty brick, and a
growth pass pushes about 230,000 children into its queue, which the visit
budget does not count.
