//! Workarounds for dx 0.7.10 on Windows. Each one was found by a failed link
//! (see decision 0002). When dx is upgraded, delete this module and
//! `link_shim`, then run `cargo xtask hot --smoke` to see what still breaks.
//!
//! 1. dx cannot be handed `rust-lld` by name (it picks how to drive a linker
//!    from its file name) and passes linkers an unquoted response file, so any
//!    argument with a space (`/LIBPATH:C:\Program Files\...`) breaks the link.
//!    The linker dx sees is this program acting as `link_shim`, which quotes
//!    the arguments and forwards to the toolchain's `rust-lld -flavor link`.
//!    `lld-link` is required anyway: dx's "fat binary" archive is GNU-format,
//!    which MSVC's `link.exe` rejects (LNK4003).
//! 2. dx runs the fat-binary link itself, outside rustc, without the `LIB`
//!    paths rustc provides (`ntdll.lib` not found), so dx is started with the
//!    variables `vcvars64.bat` sets.
//! 3. dx reuses its cached fat archive without the standard-library rlibs that
//!    belong with it (undefined `__rustc::rust_panic`), so the cache is deleted
//!    before every start.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::link_shim;

const VSWHERE: &str = r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe";

/// Prepares `dx` (workarounds 1 and 2) and the target folder (workaround 3).
pub(super) fn prepare(command: &mut Command, root: &Path) -> Result<(), String> {
    remove_stale_fat_archives(&target_dir(root));
    let shim = std::env::current_exe().map_err(|e| format!("xtask cannot find itself: {e}"))?;
    let shim = if shim.to_string_lossy().contains(' ') {
        short_path(&shim).ok_or_else(|| {
            format!(
                "dx cannot use a linker whose path has spaces, and 8.3 short names are off: {}",
                shim.display()
            )
        })?
    } else {
        shim
    };
    command
        .envs(visual_studio_environment())
        .env("CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER", shim)
        .env(link_shim::REAL_LINKER_ENV, rust_lld()?);
    Ok(())
}

/// The cargo target folder: `CARGO_TARGET_DIR`, or `target` in the workspace.
fn target_dir(root: &Path) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"))
}

/// Deletes `libdeps-*.a` in `<target>/<triple>/<profile>/`, where dx keeps them.
fn remove_stale_fat_archives(target: &Path) {
    let Ok(triples) = std::fs::read_dir(target) else {
        return;
    };
    let profiles = triples
        .flatten()
        .filter_map(|triple| std::fs::read_dir(triple.path()).ok())
        .flatten()
        .flatten();
    for profile in profiles {
        let Ok(files) = std::fs::read_dir(profile.path()) else {
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

/// The toolchain's own `rust-lld.exe`.
fn rust_lld() -> Result<PathBuf, String> {
    let output = Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .map_err(|e| format!("could not run rustc: {e}"))?;
    let sysroot = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    let lld = sysroot.join(r"lib\rustlib\x86_64-pc-windows-msvc\bin\rust-lld.exe");
    if lld.exists() {
        Ok(lld)
    } else {
        Err(format!(
            "the toolchain has no rust-lld at {}",
            lld.display()
        ))
    }
}

/// The variables `vcvars64.bat` adds or changes, as a "Developer Command
/// Prompt" has them. Only the differences are returned, so nothing else in the
/// environment is touched or printed. Empty, with a warning, without Visual Studio.
fn visual_studio_environment() -> Vec<(String, String)> {
    let Some(vcvars) = vswhere_find(r"VC\Auxiliary\Build\vcvars64.bat") else {
        eprintln!("warning: vcvars64.bat not found; dx may fail to link (LNK1104)");
        return Vec::new();
    };
    // `/U` makes `set` print UTF-16, so names like `Björn` survive. cmd does
    // not understand Rust's `\"` escaping, so the line goes in verbatim.
    let mut cmd = Command::new("cmd");
    let line = format!("/U /C call \"{}\" >nul && set", vcvars.display());
    #[cfg(windows)]
    std::os::windows::process::CommandExt::raw_arg(&mut cmd, line);
    #[cfg(not(windows))]
    cmd.arg(line);
    let output = match cmd.output() {
        Ok(o) if o.status.success() => o.stdout,
        _ => {
            eprintln!("warning: running vcvars64.bat failed; dx may fail to link");
            return Vec::new();
        }
    };
    let current: HashMap<String, String> = std::env::vars()
        .map(|(k, v)| (k.to_ascii_uppercase(), v))
        .collect();
    parse_environment(&decode_utf16le(&output))
        .into_iter()
        .filter(|(name, value)| current.get(&name.to_ascii_uppercase()) != Some(value))
        .collect()
}

fn vswhere_find(pattern: &str) -> Option<PathBuf> {
    let output = Command::new(VSWHERE)
        .args(["-latest", "-products", "*", "-find", pattern])
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.exists())
}

fn decode_utf16le(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    String::from_utf16_lossy(&units)
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
    (!short.is_empty() && !short.contains(' ')).then(|| PathBuf::from(short))
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn utf16_output_keeps_non_ascii_names() {
        let bytes: Vec<u8> = "USERNAME=Björn\r\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(
            parse_environment(&decode_utf16le(&bytes)),
            [("USERNAME".to_owned(), "Björn".to_owned())]
        );
    }

    #[test]
    fn only_fat_archives_are_removed() {
        let target = std::env::temp_dir().join(format!("xtask_fat_{}", std::process::id()));
        let profile = target.join("x86_64-pc-windows-msvc").join("desktop-dev");
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::write(profile.join("libdeps-1234abcd.a"), "x").unwrap();
        std::fs::write(profile.join("world.exe"), "x").unwrap();
        remove_stale_fat_archives(&target);
        assert!(!profile.join("libdeps-1234abcd.a").exists());
        assert!(profile.join("world.exe").exists());
        std::fs::remove_dir_all(&target).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn the_toolchain_has_rust_lld() {
        assert!(rust_lld().unwrap().ends_with("rust-lld.exe"));
    }

    #[cfg(windows)]
    #[test]
    fn the_visual_studio_environment_provides_the_library_paths() {
        let vars = visual_studio_environment();
        let lib = vars.iter().find(|(k, _)| k.eq_ignore_ascii_case("LIB"));
        assert!(
            lib.is_some_and(|(_, v)| v.contains("um\\x64")),
            "no Windows SDK libs in LIB"
        );
        // Variables vcvars does not change are not passed on.
        assert!(
            !vars
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("USERPROFILE"))
        );
    }
}
