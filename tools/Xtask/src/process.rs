//! Running other programs, with the command line printed first.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Runs `command` to completion, failing on a nonzero exit.
pub fn run(command: &mut Command) -> Result<(), String> {
    println!("$ {}", describe(command));
    let status = command.status().map_err(|e| {
        format!(
            "could not start {}: {e}",
            command.get_program().to_string_lossy()
        )
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "{} failed with {status}",
            command.get_program().to_string_lossy()
        ))
    }
}

/// Whether `cargo <subcommand> --version` works, i.e. the tool is installed.
pub fn cargo_tool_installed(subcommand: &str) -> bool {
    Command::new("cargo")
        .args([subcommand, "--version"])
        .output()
        .is_ok_and(|o| o.status.success())
}

/// The workspace root, wherever xtask was started from inside it.
pub fn workspace_root() -> PathBuf {
    // tools/Xtask -> tools -> the workspace root.
    let xtask = Path::new(env!("CARGO_MANIFEST_DIR"));
    xtask
        .parent()
        .and_then(Path::parent)
        .expect("xtask lives two levels below the workspace root")
        .to_path_buf()
}

pub fn cargo() -> Command {
    Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
}

pub fn describe(command: &Command) -> String {
    let mut parts = vec![command.get_program().to_string_lossy().into_owned()];
    parts.extend(command.get_args().map(|a| a.to_string_lossy().into_owned()));
    for (key, value) in command.get_envs() {
        if let Some(value) = value {
            parts.insert(
                0,
                format!("{}={}", key.to_string_lossy(), value.to_string_lossy()),
            );
        }
    }
    parts.join(" ")
}
