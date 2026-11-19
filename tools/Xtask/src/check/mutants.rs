//! `xtask mutants`: mutation testing of World_Core with cargo-mutants.
//!
//! What is mutated, the test runner and the timeouts live in
//! `.cargo/mutants.toml`, which cargo-mutants reads by itself. The package is
//! named here because the workspace's default member is the game, which
//! cargo-mutants would otherwise pick. Results go to `mutants.out/`.

use crate::process::{self, cargo, cargo_tool_installed, workspace_root};

/// The package whose code is mutated and whose tests must catch the mutants.
const PACKAGE: &str = "world_core";

pub fn run(extra: &[String]) -> Result<(), String> {
    if !cargo_tool_installed("mutants") {
        return Err(
            "cargo-mutants is not installed: `cargo install cargo-mutants --locked`".into(),
        );
    }
    let mut command = cargo();
    command
        .current_dir(workspace_root())
        .args(["mutants", "--package", PACKAGE])
        .args(extra);
    process::run(&mut command)?;
    println!("\nResults: mutants.out/ (caught.txt, missed.txt, timeout.txt, unviable.txt)");
    Ok(())
}
