# World

A fun game.

## What you need

- **Rust.** Install [rustup](https://rustup.rs). The repo pins its own Rust version in `rust-toolchain.toml`, and rustup downloads it the first time you build.
- **Windows:** Visual Studio Build Tools with the "Desktop development with C++" workload.
- **macOS:** Xcode command line tools (`xcode-select --install`).
- **Git LFS**, for the large asset files: [git-lfs.com](https://git-lfs.com).
- A GPU with DirectX 12, Vulkan or Metal.

## Get the code

```bash
git clone https://github.com/tonytranrp/World.git
cd World
git lfs install
git lfs pull
```

## Build

```bash
cargo build
```

The first build compiles about 380 crates and takes around 12 minutes. After that, builds only recompile what you changed.

## Run the game

```bash
cargo run
```

This opens the game window. Run it from the repository root. The game picks the best graphics backend and GPU by itself.

Options go after `--`:

```bash
cargo run -- --list-adapters        # list your GPUs and what each can do, then exit
cargo run -- --backend vulkan       # force dx12, vulkan or metal
cargo run -- --mac-floor            # low-spec mode: 720p, small memory budget, 4 threads
cargo run -- --exit-after-frames 300  # close by itself after 300 frames
cargo run -- --help                 # every option
```

## Test

Install the test runner once:

```bash
cargo install cargo-nextest --locked
```

Then:

```bash
cargo xtask test          # fast tests, run before every commit
cargo xtask test all      # also the tests that render on a GPU adapter
cargo xtask test full     # the big run: one million random edits
cargo xtask check         # formatting, lints and the fast tests
cargo xtask bench --save main            # save benchmark speeds as a baseline
cargo xtask bench --check main           # fail if anything got more than 10% slower
cargo xtask golden --update              # rewrite reference images after you checked them
cargo xtask coverage                     # which lines the tests miss (needs cargo-llvm-cov)
```

Plain `cargo test --workspace` also works.

## Release build

```bash
cargo run --release --no-default-features
```

The normal build links Bevy dynamically and watches assets for changes, which is faster to work with. `--no-default-features` turns that off for a release build.

## Where things are

| Folder | What is in it |
| --- | --- |
| `app/` | The game executable |
| `crates/` | The engine and game code |
| `assets/` | Shaders, config, models, sounds |
| `docs/` | The engine plan and design notes |
| `tools/`, `packaging/`, `scripts/` | Build helpers and packaging |

Every folder has its own `README.md`. The engine plan is in `docs/plans/engine-plan.md`.
