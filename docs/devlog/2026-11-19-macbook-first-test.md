# 2026-11-19: First test on the 2019 MacBook Pro

The 2019 16-inch MacBook Pro is the performance floor (Intel CPU, AMD Radeon
Pro 5300M or better, Metal). Nothing has run on it yet. This entry is the
checklist; fill in the **Result** lines as you go and paste anything that
fails, so the fixes start from real output.

## 1. Set up the Mac

```bash
xcode-select --install
```

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Git LFS, for the golden images (installer from git-lfs.com, or with Homebrew):

```bash
brew install git-lfs
```

```bash
git clone https://github.com/tonytranrp/World.git
```

```bash
cd World && git lfs install && git lfs pull && git switch hot-reload-engine
```

```bash
cargo install cargo-nextest --locked
```

**Result:** macOS version (Apple menu > About This Mac): ____. GPU and memory: ____.

## 2. Which GPU does the game see?

```bash
cargo run -- --list-adapters
```

Expected: the AMD Radeon on Metal, tier B. The Intel UHD 630 may also appear.
The first build compiles everything and may take 20-30 minutes on this CPU.

**Result:** ____

## 3. Tests

```bash
cargo xtask test
```

Metal has no software adapter, so the GPU tests need the real GPU:

```bash
WORLD_TEST_ADAPTER=gpu cargo xtask test gpu
```

The golden images were made on Windows' software renderer. A small colour
difference on the Radeon is expected; a large one is a finding.

**Result:** ____

## 4. The game, with live reloading

```bash
cargo xtask dev
```

- [ ] A window opens showing the blue sky gradient.
- [ ] F1 opens the entity inspector; F2 the config editor; F5 logs `code reload #1`.
- [ ] Change `gravity_m_s2` in `assets/config/physics.ron`, save: the log says `config physics: applied`.
- [ ] Save a broken value (`step_hz: 5000.0`): the log and F2 show it was rejected; the game keeps running.
- [ ] Change `ZENITH` in `assets/shaders/common/sky.wesl`, save: the sky changes without a restart.

```bash
cargo run -- --mac-floor
```

**Result:** ____

## 5. Rust code hot-patching

```bash
cargo install dioxus-cli --version 0.7.10 --locked
```

```bash
cargo xtask hot --smoke
```

This has only ever run on Windows. If it fails, paste the last lines it prints
and `target/hot-smoke.log`.

**Result:** ____

## What happens next

Findings get fixed on the `hot-reload-engine` branch; PR #1 merges once this
checklist passes (decision 0003). Then the `.app` packaging work starts.
