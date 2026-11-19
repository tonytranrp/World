//! `xtask golden`: check golden files, or regenerate them on purpose.

use crate::process::{self, cargo, cargo_tool_installed};

/// Runs the cross-crate suites that own golden files. With `update`, rewrites
/// the golden files instead of comparing, then shows what changed so the
/// difference can be reviewed before committing.
pub fn run(update: bool) -> Result<(), String> {
    let mut command = cargo();
    if cargo_tool_installed("nextest") {
        command.args(["nextest", "run", "-p", "world_tests", "--no-fail-fast"]);
    } else {
        command.args(["test", "-p", "world_tests", "--no-fail-fast"]);
    }
    // GPU tests must run, not skip, or a golden image could be left stale.
    command.env("WORLD_TEST_REQUIRE_GPU", "1");
    if update {
        command.env("UPDATE_GOLDEN", "1");
    }
    process::run(&mut command)?;

    if update {
        println!("\nGolden files changed by this run (review before committing):");
        let mut status = std::process::Command::new("git");
        status.args([
            "status",
            "--short",
            "--",
            "tests/golden",
            "tests/replays",
            "tests/scenes",
        ]);
        process::run(&mut status)?;
    }
    Ok(())
}
