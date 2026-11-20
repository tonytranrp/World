//! `xtask bench --instructions`: instruction counts with gungraun.
//!
//! gungraun runs each benchmark once under Valgrind's Callgrind and counts
//! instructions, which barely move between runs (about 0.1% on shared CI
//! runners, where wall time varies several-fold), so small regressions are
//! visible. Valgrind only runs on Linux.

use std::process::Command;

use crate::process::{self, cargo, workspace_root};

/// The crate and bench target that hold the gungraun benchmarks.
const PACKAGE: &str = "world_core";
const BENCH: &str = "instructions";

pub fn run(save: Option<&str>, check: Option<&str>, threshold_percent: f64) -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err(
            "instruction-count benchmarks need Linux with Valgrind (gungraun uses Callgrind). \
             On Windows run them in WSL; CI runs them in the `bench-instructions` job."
                .into(),
        );
    }
    let version = gungraun_version()?;
    if !tool_works("valgrind") {
        return Err(
            "valgrind is not installed or not on PATH: install it with your package manager \
             (`sudo apt-get install valgrind`)."
                .into(),
        );
    }
    if !tool_works("gungraun-runner") {
        return Err(format!(
            "gungraun-runner is not installed or not on PATH; it must match the gungraun crate: \
             `cargo install gungraun-runner --version {version} --locked`"
        ));
    }
    let mut command = cargo();
    command
        .args(["bench", "-p", PACKAGE, "--bench", BENCH, "--"])
        .args(gungraun_args(save, check, threshold_percent))
        // gungraun needs symbols to find the benchmark functions in Callgrind's
        // output; the release profile the bench profile inherits has none.
        .env("CARGO_PROFILE_BENCH_DEBUG", "true");
    process::run(&mut command)
}

/// Runner arguments for the requested baseline action. `--check` compares
/// against a saved baseline and fails (exit code 3) when any benchmark's
/// instruction count grew by more than the threshold.
fn gungraun_args(save: Option<&str>, check: Option<&str>, threshold_percent: f64) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(name) = save {
        args.push(format!("--save-baseline={name}"));
    }
    if let Some(name) = check {
        args.push(format!("--baseline={name}"));
        args.push(format!("--callgrind-limits=ir={threshold_percent}%"));
    }
    args
}

fn tool_works(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// The locked version of the gungraun crate, which gungraun-runner must match.
fn gungraun_version() -> Result<String, String> {
    let lock = workspace_root().join("Cargo.lock");
    let text = std::fs::read_to_string(&lock).map_err(|e| format!("{}: {e}", lock.display()))?;
    locked_version(&text, "gungraun").ok_or_else(|| "gungraun is not in Cargo.lock".to_owned())
}

/// The version of package `name` in the text of a Cargo.lock.
fn locked_version(lock: &str, name: &str) -> Option<String> {
    let header = format!("name = \"{name}\"");
    let mut lines = lock.lines();
    lines.find(|line| line.trim() == header)?;
    let version = lines.next()?.trim().strip_prefix("version = \"")?;
    Some(version.strip_suffix('"')?.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_compares_and_limits_instruction_growth() {
        assert_eq!(
            gungraun_args(None, Some("main"), 5.0),
            ["--baseline=main", "--callgrind-limits=ir=5%"]
        );
    }

    #[test]
    fn save_and_check_together_compare_then_store() {
        assert_eq!(
            gungraun_args(Some("pr"), Some("main"), 2.5),
            [
                "--save-baseline=pr",
                "--baseline=main",
                "--callgrind-limits=ir=2.5%"
            ]
        );
        assert!(gungraun_args(None, None, 5.0).is_empty());
    }

    #[test]
    fn reads_a_locked_version() {
        let lock = "[[package]]\nname = \"gungraun-runner\"\nversion = \"9.9.9\"\n\n\
                    [[package]]\nname = \"gungraun\"\nversion = \"0.20.0\"\n";
        assert_eq!(locked_version(lock, "gungraun").as_deref(), Some("0.20.0"));
        assert_eq!(locked_version(lock, "missing"), None);
    }

    #[test]
    fn the_workspace_locks_gungraun() {
        assert!(gungraun_version().is_ok());
    }
}
