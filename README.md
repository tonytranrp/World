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

## Change the game while it runs

```bash
cargo xtask dev           # same as cargo run, with the developer panels
```

Saving a file under `assets/` updates the running game: shaders, and the tuning in `assets/config/` (materials, physics). A file with a mistake is ignored, the game keeps the last good version, and the reason shows in the log and in the F2 panel.

| Key | Panel |
| --- | --- |
| F1 | Inspect every entity |
| F2 | Config editor: edit, Apply (try it live) or Save (write the file) |
| F5 | Re-run setup code |

To also change Rust code while the game runs, install the dx tool once (the version must match, `cargo xtask hot` tells you if it does not):

```bash
cargo install dioxus-cli --version 0.7.10 --locked
```

```bash
cargo xtask hot           # run under dx; saving a .rs file patches the running game
cargo xtask hot --smoke   # check that code patching works on this machine
```

The first `hot` run builds the game a second way and takes about 20 minutes; after that the game starts in under a minute. An edit to system code reaches the running game in about 25 seconds on an RTX 4070 laptop, without losing the game's state. Changing the fields of a struct still needs a restart.

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
cargo xtask test reload   # only the hot-reload tests, shader reload included
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

The normal build links Bevy dynamically, watches assets for changes and includes the developer panels, which is faster to work with. `--no-default-features` turns all of that off for a release build.

## Where things are

| Folder | What is in it |
| --- | --- |
| `app/` | The game executable |
| `crates/` | The engine and game code |
| `assets/` | Shaders, config, models, sounds |
| `docs/` | The engine plan and design notes |
| `tools/`, `packaging/`, `scripts/` | Build helpers and packaging |

Every folder has its own `README.md`. The engine plan is in `docs/plans/engine-plan.md`.
