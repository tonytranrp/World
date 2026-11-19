//! `xtask check`, `xtask coverage` and `xtask miri`.

use crate::process::{self, cargo, cargo_tool_installed};
use crate::testing::{self, TestProfile};

/// Formatting, clippy with warnings as errors, then the fast tests.
pub fn run() -> Result<(), String> {
    process::run(cargo().args(["fmt", "--all", "--check"]))?;
    process::run(cargo().args([
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ]))?;
    testing::run(TestProfile::Fast, &[])
}

/// Coverage as an HTML report in `target/llvm-cov/html`.
pub fn coverage() -> Result<(), String> {
    if !cargo_tool_installed("llvm-cov") {
        return Err(
            "cargo-llvm-cov is not installed: `cargo install cargo-llvm-cov --locked` and \
                    `rustup component add llvm-tools-preview`"
                .into(),
        );
    }
    let mut command = cargo();
    command
        .args([
            "llvm-cov",
            "--workspace",
            "--html",
            "--ignore-filename-regex",
            r"[\\/](tests|benches)[\\/]",
        ])
        .env("WORLD_TEST_PROFILE", "fast")
        // Skip GPU tests: instrumented software rendering is very slow.
        .args(["--", "--skip", "gpu_"]);
    process::run(&mut command)?;
    println!("\nReport: target/llvm-cov/html/index.html");
    Ok(())
}

/// Miri checks the crates that may contain unsafe code, on the nightly toolchain.
/// Property tests run at a tiny scale there because Miri is very slow.
pub fn miri() -> Result<(), String> {
    // Cargo run by `cargo xtask` is the pinned toolchain's own binary, which does
    // not understand `+nightly`. Go through the rustup proxy and pick the toolchain
    // by environment instead.
    let mut command = std::process::Command::new("cargo");
    command
        .env_remove("CARGO")
        .env("RUSTUP_TOOLCHAIN", "nightly")
        .args(["miri", "test", "-p", "world_core", "-p", "world_testkit"])
        .env("PROPTEST_CASES", "4")
        .env("MIRIFLAGS", "-Zmiri-disable-isolation")
        .args(["--no-fail-fast", "--", "--skip", "golden", "--skip", "gpu_"]);
    process::run(&mut command)
}
