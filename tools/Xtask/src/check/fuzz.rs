//! `xtask fuzz`: cargo-fuzz (libFuzzer) on World_Core's decoder.
//!
//! The fuzz project in `crates/Foundation/World_Core/fuzz` is its own
//! workspace and needs the nightly toolchain, so the main build never does.
//! Each run first writes valid encodings as seeds, then fuzzes each target for
//! a bounded time. A crash fails the command and leaves the input in
//! `fuzz/artifacts/<target>/`.

use std::path::Path;
use std::process::Command;

use crate::process::{self, workspace_root};

/// The fuzz targets, as named in the fuzz project's Cargo.toml.
pub const TARGETS: &[&str] = &["decode", "round_trip"];

/// The fuzz project, relative to the workspace root.
const FUZZ_DIR: &str = "crates/Foundation/World_Core/fuzz";

/// Largest input libFuzzer generates. Encodings of small trees are a few KiB;
/// this leaves room for big ones without slowing every run down.
const MAX_INPUT_BYTES: u32 = 64 * 1024;

pub fn run(target: Option<&str>, seconds: u64) -> Result<(), String> {
    let fuzz_dir = workspace_root().join(FUZZ_DIR);
    if !nightly_cargo()
        .args(["fuzz", "--version"])
        .output()
        .is_ok_and(|o| o.status.success())
    {
        return Err("cargo-fuzz on the nightly toolchain is required: \
             `rustup toolchain install nightly --profile minimal --no-self-update` and \
             `cargo install cargo-fuzz --locked`"
            .into());
    }
    if cfg!(windows) {
        eprintln!(
            "note: cargo-fuzz support on Windows is experimental; Linux, macOS or WSL work best."
        );
    }
    let targets: Vec<&str> = match target {
        Some(name) => vec![name],
        None => TARGETS.to_vec(),
    };

    // Seeds go into each target's corpus folder, where libFuzzer also keeps
    // the inputs it finds interesting; rewriting the seeds is harmless.
    let corpus = |name: &str| fuzz_dir.join("corpus").join(name);
    let mut seeds = nightly_cargo();
    seeds
        .arg("run")
        .args(["--quiet", "--release", "--manifest-path"])
        .arg(fuzz_dir.join("Cargo.toml"))
        .args(["--bin", "make_seeds", "--"])
        .args(targets.iter().map(|t| corpus(t)));
    process::run(&mut seeds)?;

    // cargo-fuzz defaults to the target it was itself built for, and the
    // prebuilt binaries CI installs are musl, where sanitizers cannot link.
    let host = host_triple()?;
    let mut failed = Vec::new();
    for name in &targets {
        let mut command = nightly_cargo();
        command
            .args(["fuzz", "run", "--target", &host, "--fuzz-dir"])
            .arg(&fuzz_dir)
            // Overflow and other debug assertions in World_Core are bugs too.
            .args(["--debug-assertions", name, "--"])
            .args(libfuzzer_args(seconds));
        if let Err(e) = process::run(&mut command) {
            failed.push(format!(
                "{name}: {e}\n  {}",
                reproduce_hint(&fuzz_dir, name)
            ));
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!("fuzzing found problems:\n{}", failed.join("\n")))
    }
}

/// libFuzzer options: stop after `seconds`, cap the input size.
fn libfuzzer_args(seconds: u64) -> Vec<String> {
    vec![
        format!("-max_total_time={seconds}"),
        format!("-max_len={MAX_INPUT_BYTES}"),
    ]
}

fn reproduce_hint(fuzz_dir: &Path, target: &str) -> String {
    let artifacts = fuzz_dir.join("artifacts").join(target);
    format!(
        "crash inputs are in {}; replay one with \
         `cargo +nightly fuzz run --fuzz-dir {} {target} <file>`",
        artifacts.display(),
        fuzz_dir.display()
    )
}

/// Cargo through the rustup proxy on the nightly toolchain. The cargo that
/// runs `cargo xtask` is the pinned toolchain's own binary, which does not
/// understand `+nightly` (as for Miri).
/// The nightly compiler's host target, such as `x86_64-unknown-linux-gnu`.
fn host_triple() -> Result<String, String> {
    let output = Command::new("rustc")
        .env("RUSTUP_TOOLCHAIN", "nightly")
        .arg("-vV")
        .output()
        .map_err(|e| format!("running `rustc -vV` on nightly: {e}"))?;
    parse_host(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| "`rustc -vV` printed no `host:` line".into())
}

fn parse_host(version: &str) -> Option<String> {
    version
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .map(|host| host.trim().to_owned())
}

fn nightly_cargo() -> Command {
    let mut command = Command::new("cargo");
    command
        .env_remove("CARGO")
        .env("RUSTUP_TOOLCHAIN", "nightly");
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn libfuzzer_runs_are_bounded() {
        assert_eq!(
            libfuzzer_args(300),
            ["-max_total_time=300", "-max_len=65536"]
        );
    }

    #[test]
    fn the_host_comes_from_rustc_version_output() {
        let version = "rustc 1.99.0-nightly (abc 2026-11-01)\nbinary: rustc\n\
                       host: x86_64-unknown-linux-gnu\nrelease: 1.99.0-nightly\n";
        assert_eq!(
            parse_host(version).as_deref(),
            Some("x86_64-unknown-linux-gnu")
        );
        assert_eq!(parse_host("rustc 1.99.0\n"), None);
    }

    #[test]
    fn every_target_is_declared_in_the_fuzz_project() {
        let manifest = workspace_root().join(FUZZ_DIR).join("Cargo.toml");
        let text = std::fs::read_to_string(&manifest).unwrap();
        for target in TARGETS {
            assert!(
                text.contains(&format!("name = \"{target}\"")),
                "{target} is not a [[bin]] in {}",
                manifest.display()
            );
        }
    }
}
