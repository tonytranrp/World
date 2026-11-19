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
    remove_stale_fat_archives(&root);
    if smoke {
        return smoke::run(&root, args);
    }
    let mut command = dx_serve(&root, args, true);
    process::run(&mut command)
}

/// dx caches the hot-patch "fat archive" (`libdeps-<hash>.a`) next to the
/// game, but 0.7.10 reuses it without the list of standard-library rlibs that
/// belongs with it, so a second run links without std (undefined
/// `__rustc::rust_panic` and friends). Deleting it makes dx rebuild both.
pub(super) fn remove_stale_fat_archives(root: &Path) {
    let Ok(target_dirs) = std::fs::read_dir(root.join("target")) else {
        return;
    };
    for profile_dir in target_dirs
        .flatten()
        .filter_map(|triple| std::fs::read_dir(triple.path()).ok())
        .flatten()
        .flatten()
    {
        let Ok(files) = std::fs::read_dir(profile_dir.path()) else {
            continue;
        };
        for file in files.flatten() {
            let name = file.file_name().to_string_lossy().into_owned();
            if name.starts_with("libdeps-") && name.ends_with(".a") {
                let _ = std::fs::remove_file(file.path());
            }
        }
    }
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
        // dx 0.7.10 cannot drive rust-lld, which .cargo/config.toml selects,
        // and passes MSVC's link.exe an unquoted response file. So the linker
        // dx sees is this program acting as a shim (see `link_shim`), which
        // quotes the arguments and runs MSVC's linker by full path: Git for
        // Windows ships an unrelated `link.exe` that PATH would find first.
        let shim = std::env::current_exe().expect("xtask knows its own path");
        // dx also runs the linker itself (the hot-patch "fat binary"), outside
        // rustc, so it needs the Visual Studio environment (LIB, LIBPATH, PATH)
        // that rustc would otherwise provide.
        command.envs(msvc_environment());
        command
            .env(
                "CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER",
                short_path(&shim).unwrap_or(shim),
            )
            .env(link_shim::REAL_LINKER_ENV, real_linker());
    }
    if !args.is_empty() {
        command.args(["--args", &args.join(" ")]);
    }
    command
}

/// The linker the shim forwards to: the toolchain's own `rust-lld` (used as
/// `lld-link`), because dx's hot-patch "fat binary" bundles dependencies in a
/// GNU-format archive that MSVC's `link.exe` rejects (LNK4003). Falls back to
/// MSVC's linker if the toolchain has no `rust-lld`.
fn real_linker() -> PathBuf {
    let sysroot = Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .ok()
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()));
    sysroot
        .map(|root| root.join(r"lib\rustlib\x86_64-pc-windows-msvc\bin\rust-lld.exe"))
        .filter(|lld| lld.exists())
        .unwrap_or_else(msvc_linker)
}

/// The full path of MSVC's `link.exe`, found with Visual Studio's `vswhere`.
/// Falls back to the bare name, with a warning, when Visual Studio is not found.
fn msvc_linker() -> PathBuf {
    let vswhere =
        Path::new(r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe");
    let found = Command::new(vswhere)
        .args(["-latest", "-products", "*", "-find"])
        .arg(r"VC\Tools\MSVC\**\bin\Hostx64\x64\link.exe")
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .map(PathBuf::from)
        })
        .filter(|p| p.exists());
    match found {
        Some(path) => path,
        None => {
            eprintln!("warning: MSVC link.exe not found with vswhere; relying on PATH");
            PathBuf::from("link.exe")
        }
    }
}

/// The variables `vcvars64.bat` sets: what a "Developer Command Prompt" has.
/// Empty, with a warning, when Visual Studio is not found.
fn msvc_environment() -> Vec<(String, String)> {
    let vswhere =
        Path::new(r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe");
    let vcvars = Command::new(vswhere)
        .args(["-latest", "-products", "*", "-find"])
        .arg(r"VC\Auxiliary\Build\vcvars64.bat")
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .map(PathBuf::from)
        })
        .filter(|p| p.exists());
    let Some(vcvars) = vcvars else {
        eprintln!("warning: vcvars64.bat not found; dx may fail to link (LNK1104)");
        return Vec::new();
    };
    let mut cmd = Command::new("cmd");
    let line = format!("/C call \"{}\" >nul && set", vcvars.display());
    // cmd does not understand Rust's `\"` escaping; hand it the line verbatim.
    #[cfg(windows)]
    std::os::windows::process::CommandExt::raw_arg(&mut cmd, line);
    #[cfg(not(windows))]
    cmd.arg(line);
    let output = cmd.output();
    match output {
        Ok(o) if o.status.success() => parse_environment(&String::from_utf8_lossy(&o.stdout)),
        _ => {
            eprintln!("warning: running vcvars64.bat failed; dx may fail to link");
            Vec::new()
        }
    }
}

/// `NAME=value` lines, as printed by `set`.
fn parse_environment(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| line.split_once('='))
        .filter(|(name, _)| !name.is_empty())
        .map(|(name, value)| (name.to_owned(), value.trim_end_matches('\r').to_owned()))
        .collect()
}

/// The 8.3 short form of an existing path (`C:\PROGRA~1\...`), which has no
/// spaces. `None` when short names are disabled on the volume.
fn short_path(path: &Path) -> Option<PathBuf> {
    let script = format!(
        "(New-Object -ComObject Scripting.FileSystemObject).GetFile('{}').ShortPath",
        path.display().to_string().replace('\'', "''")
    );
    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .ok()?;
    let short = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if short.is_empty() || short.contains(' ') {
        eprintln!(
            "warning: no space-free short path for {}; dx may fail to link",
            path.display()
        );
        return None;
    }
    Some(PathBuf::from(short))
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
        let pos = args.iter().position(|a| a == "--args").unwrap();
        assert_eq!(args[pos + 1], "--mac-floor");
    }

    #[cfg(windows)]
    #[test]
    fn the_real_linker_is_the_toolchains_rust_lld() {
        let linker = real_linker();
        assert!(linker.ends_with("rust-lld.exe"), "{}", linker.display());
    }

    #[cfg(windows)]
    #[test]
    fn the_msvc_linker_is_found_by_full_path() {
        let linker = msvc_linker();
        assert!(linker.is_absolute(), "{}", linker.display());
        assert!(linker.exists());
        assert!(linker.ends_with("x64/link.exe") || linker.ends_with(r"x64\link.exe"));
    }

    #[test]
    fn set_output_is_parsed_into_variables() {
        let vars = parse_environment("LIB=C:\\a;C:\\b\r\nPath=C:\\x=y\r\n=C:=C:\\\r\n");
        assert_eq!(
            vars,
            [
                ("LIB".to_owned(), "C:\\a;C:\\b".to_owned()),
                ("Path".to_owned(), "C:\\x=y".to_owned())
            ]
        );
    }

    #[cfg(windows)]
    #[test]
    fn the_visual_studio_environment_provides_the_library_paths() {
        let vars = msvc_environment();
        let lib = vars.iter().find(|(k, _)| k.eq_ignore_ascii_case("LIB"));
        assert!(
            lib.is_some_and(|(_, v)| v.contains("um\\x64")),
            "no Windows SDK libs in LIB"
        );
    }

    #[test]
    fn the_lock_file_pins_subsecond() {
        assert!(locked_version(&workspace_root(), SUBSECOND).is_ok());
    }
}
