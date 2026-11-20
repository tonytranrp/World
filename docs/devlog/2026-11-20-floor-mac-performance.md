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
