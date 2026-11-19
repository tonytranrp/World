//! `xtask dev` and `xtask hot`: running the game with live reloading.
//!
//! - `dev`: plain `cargo run` with the `dev` feature: dynamic linking, asset
//!   and config hot reload, developer panels. Rust changes need a restart.
//! - `hot`: `dx serve --hot-patch` with the `hotpatch` feature, so Rust edits
//!   to systems apply while the game keeps running.
//! - `hot --smoke`: proves the whole hot-patch chain works on this machine. It
//!   starts the game under dx, rewrites the probe in `app/src/startup/hot_probe.rs`,
//!   waits for the patched value in the game's log, then restores the file.

pub mod link_shim;
mod smoke;
mod windows;

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::process::{self, cargo};

/// The Rust edits dx can patch are compiled against this exact protocol
/// version, so the installed `dx` must match the `subsecond` crate in the lock file.
const SUBSECOND: &str = "subsecond";

pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root exists")
}

/// `cargo run` with the default `dev` feature.
pub fn dev(args: &[String]) -> Result<(), String> {
    let mut command = cargo();
    command.args(["run", "--package", "world", "--"]).args(args);
    process::run(&mut command)
}

/// `dx serve --hot-patch`, interactive, until the user quits.
pub fn hot(args: &[String], smoke: bool) -> Result<(), String> {
    let root = workspace_root();
    let wanted = locked_version(&root, SUBSECOND)?;
    check_dx(&wanted)?;
    if smoke {
        return smoke::run(&root, args);
    }
    let mut command = dx_serve(&root, args, true);
    prepare(&mut command, &root)?;
    process::run(&mut command)
}

/// Platform workarounds that `dx serve` needs before it starts.
pub(super) fn prepare(command: &mut Command, root: &Path) -> Result<(), String> {
    if cfg!(windows) {
        windows::prepare(command, root)?;
    }
    Ok(())
}

/// The `dx serve` command line shared by `hot` and the smoke test.
pub(super) fn dx_serve(root: &Path, args: &[String], interactive: bool) -> Command {
    let mut command = Command::new("dx");
    command
        .current_dir(root)
        .args([
            "serve",
            "--hot-patch",
            "--package",
            "world",
            "--bin",
            "world",
            // `dev` turns on dynamic linking, which breaks patching on Windows.
            "--no-default-features",
            "--features",
            "hotpatch",
            "--interactive",
            if interactive { "true" } else { "false" },
            "--always-on-top",
            "false",
        ])
        // dx runs the game from its own bundle folder; point Bevy at our assets.
        .env("BEVY_ASSET_ROOT", root);
    if cfg!(windows) {
        // Keep a console so the game's log reaches dx, and the smoke test.
        command.args(["--windows-subsystem", "CONSOLE"]);
    }
    if !args.is_empty() {
        // One `--args=` token: as a separate value, `--mac-floor` would be read
        // by dx itself as an unknown flag. dx splits it on spaces for the game.
        command.arg(format!("--args={}", args.join(" ")));
    }
    command
}

/// The version of `package` recorded in Cargo.lock.
pub fn locked_version(root: &Path, package: &str) -> Result<String, String> {
    let lock = std::fs::read_to_string(root.join("Cargo.lock"))
        .map_err(|e| format!("could not read Cargo.lock: {e}"))?;
    parse_locked_version(&lock, package).ok_or_else(|| {
        format!("{package} is not in Cargo.lock; is the `hotpatch` feature wired up?")
    })
}

fn parse_locked_version(lock: &str, package: &str) -> Option<String> {
    let header = format!("name = \"{package}\"");
    let mut lines = lock.lines();
    while let Some(line) = lines.next() {
        if line.trim() == header {
            let version = lines.next()?.trim();
            return version
                .strip_prefix("version = \"")
                .and_then(|v| v.strip_suffix('"'))
                .map(str::to_owned);
        }
    }
    None
}

fn check_dx(wanted: &str) -> Result<(), String> {
    let install = format!("cargo install dioxus-cli --version {wanted} --locked");
    let output = Command::new("dx")
        .arg("--version")
        .output()
        .map_err(|_| format!("dx is not installed. Install it with:\n  {install}"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    match parse_dx_version(&text) {
        Some(found) if found == wanted => Ok(()),
        Some(found) => Err(format!(
            "dx {found} is installed but subsecond is {wanted}; they must match. Install with:\n  {install}"
        )),
        None => Err(format!("could not read the dx version from {text:?}")),
    }
}

/// `dioxus 0.7.10 (57d6794)` -> `0.7.10`.
fn parse_dx_version(text: &str) -> Option<String> {
    text.split_whitespace().nth(1).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_version_from_the_lock_file() {
        let lock = "[[package]]\nname = \"subsecond\"\nversion = \"0.7.10\"\nsource = \"x\"\n\n[[package]]\nname = \"subsecond-types\"\nversion = \"9.9.9\"\n";
        assert_eq!(
            parse_locked_version(lock, "subsecond").as_deref(),
            Some("0.7.10")
        );
        assert_eq!(
            parse_locked_version(lock, "subsecond-types").as_deref(),
            Some("9.9.9")
        );
        assert_eq!(parse_locked_version(lock, "bevy"), None);
    }

    #[test]
    fn reads_the_dx_version_line() {
        assert_eq!(
            parse_dx_version("dioxus 0.7.10 (57d6794)\n").as_deref(),
            Some("0.7.10")
        );
        assert_eq!(parse_dx_version(""), None);
    }

    #[test]
    fn the_hot_command_disables_dynamic_linking_and_enables_hotpatch() {
        let command = dx_serve(Path::new("."), &["--mac-floor".into()], false);
        let args: Vec<_> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let has = |a: &str| args.iter().any(|x| x == a);
        assert!(has("--hot-patch") && has("--no-default-features") && has("hotpatch"));
        assert!(has("--args=--mac-floor"), "{args:?}");
    }

    #[test]
    fn the_lock_file_pins_subsecond() {
        assert!(locked_version(&workspace_root(), SUBSECOND).is_ok());
    }
}
