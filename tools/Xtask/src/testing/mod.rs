//! `xtask test`: the test profiles.

use clap::ValueEnum;

use crate::process::{self, cargo, cargo_tool_installed};

/// Tests whose name starts with `gpu_` need a GPU adapter.
const GPU_TESTS: &str = r"test(/(^|::)gpu_/)";

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum TestProfile {
    /// Everything except GPU tests, at fast scale: for every commit.
    Fast,
    /// Only GPU tests, on the software adapter.
    Gpu,
    /// Everything, at the plan's one-million-edit scale.
    Full,
    /// Everything, at fast scale.
    All,
    /// Only the hot-reload engine: configs, code reload, shader reload (GPU),
    /// the developer panels, and the `hotpatch` feature's bridge.
    Reload,
}

/// Every test that exercises live reloading.
const RELOAD_TESTS: &str = "package(world_reload) | binary(hot_reload) | test(/ui::dev::/) \
                            | test(/rapier::tests::/) | binary(materials)";

impl TestProfile {
    fn scale(self) -> &'static str {
        match self {
            Self::Full => "full",
            _ => "fast",
        }
    }

    /// nextest filter expression, or `None` for "run everything".
    fn filter(self) -> Option<String> {
        match self {
            Self::Fast => Some(format!("not {GPU_TESTS}")),
            Self::Gpu => Some(GPU_TESTS.to_owned()),
            Self::Reload => Some(RELOAD_TESTS.to_owned()),
            Self::Full | Self::All => None,
        }
    }

    /// GPU tests may not skip when they are meant to run.
    fn requires_gpu(self) -> bool {
        // Other profiles leave the variable as the caller set it: `Reload` is a
        // local loop, and on a Mac (no software adapter) its GPU tests skip loudly.
        matches!(self, Self::Gpu | Self::Full | Self::All)
    }

    /// Whether to also build `world_reload` with its `hotpatch` feature, which
    /// the workspace build leaves off, and test the `HotPatched` bridge.
    fn tests_hotpatch_bridge(self) -> bool {
        matches!(self, Self::Full | Self::All | Self::Reload)
    }
}

pub fn run(profile: TestProfile, extra: &[String]) -> Result<(), String> {
    if cargo_tool_installed("nextest") {
        let mut command = cargo();
        command
            .args(["nextest", "run", "--workspace", "--no-fail-fast"])
            .env("WORLD_TEST_PROFILE", profile.scale());
        require_gpu_if_needed(&mut command, profile);
        if let Some(filter) = profile.filter() {
            command.args(["-E", &filter]);
        }
        process::run(command.args(extra))?;
    } else {
        eprintln!(
            "cargo-nextest is not installed; falling back to `cargo test` (install it with `cargo install cargo-nextest --locked`)."
        );
        let mut command = cargo();
        command
            .args(["test", "--workspace", "--no-fail-fast"])
            .env("WORLD_TEST_PROFILE", profile.scale());
        require_gpu_if_needed(&mut command, profile);
        if profile == TestProfile::Fast {
            // The fallback runner has no filter language; skipping by name is the closest match.
            command.args(["--", "--skip", "gpu_"]);
        }
        process::run(command.args(extra))?;
        return Ok(());
    }

    if profile.tests_hotpatch_bridge() {
        process::run(cargo().args([
            "test",
            "--package",
            "world_reload",
            "--features",
            "hotpatch",
            "--test",
            "code_reload",
        ]))?;
    }

    // nextest does not run doctests.
    if !matches!(profile, TestProfile::Gpu | TestProfile::Reload) {
        process::run(cargo().args(["test", "--doc", "--workspace"]))?;
    }
    Ok(())
}

/// Profiles that must run their GPU tests make a missing adapter a failure.
/// The rest leave `WORLD_TEST_REQUIRE_GPU` as the caller set it.
fn require_gpu_if_needed(command: &mut std::process::Command, profile: TestProfile) {
    if profile.requires_gpu() {
        command.env("WORLD_TEST_REQUIRE_GPU", "1");
    }
}
