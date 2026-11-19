//! A linker shim for `dx` on Windows.
//!
//! dx 0.7.10 links by writing every linker argument, one per line, into a
//! response file and running `<linker> @file`. It does not quote them, and
//! MSVC's `link.exe` splits response files at spaces, so any argument with a
//! space (`/LIBPATH:C:\Program Files\...`, which build scripts emit) breaks the
//! link with `LNK1104: cannot open file 'Files\...'`.
//!
//! `cargo xtask hot` names this program as the linker and puts the real linker
//! in [`REAL_LINKER_ENV`]. Every argument is forwarded; each `@file` argument is
//! replaced by a quoted copy of that file. dx's own file is left untouched,
//! because dx reads it back to build hot patches.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// Set by `xtask hot`: the real linker the shim forwards to.
pub const REAL_LINKER_ENV: &str = "WORLD_REAL_LINKER";

/// The real linker, when this process was started as the linker shim: the
/// variable is set (it is inherited by everything dx starts) and the arguments
/// look like a linker's (`@file`, `/FLAG` or `-flag`), not an xtask command.
pub fn invoked_as_shim() -> Option<PathBuf> {
    let linker = std::env::var_os(REAL_LINKER_ENV)?;
    let first = std::env::args_os().nth(1)?;
    looks_like_linker_arg(&first.to_string_lossy()).then(|| PathBuf::from(linker))
}

fn looks_like_linker_arg(arg: &str) -> bool {
    arg.starts_with(['@', '/', '-'])
}

pub fn run(linker: &Path) -> ExitCode {
    match forward(linker, std::env::args_os().skip(1).collect()) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("xtask link shim: {message}");
            ExitCode::FAILURE
        }
    }
}

fn forward(linker: &Path, args: Vec<OsString>) -> Result<ExitCode, String> {
    // Output passes straight through, so dx sees the linker's own messages.
    let status = Command::new(linker)
        .args(linker_args(linker, args)?)
        .status()
        .map_err(|e| format!("could not run {}: {e}", linker.display()))?;
    Ok(match status.code() {
        Some(0) => ExitCode::SUCCESS,
        Some(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        None => ExitCode::FAILURE,
    })
}

/// What the real linker is given: `-flavor link` first for rust-lld, then
/// every argument, with each dx response file replaced by a quoted copy.
fn linker_args(linker: &Path, args: Vec<OsString>) -> Result<Vec<OsString>, String> {
    let mut forwarded = Vec::with_capacity(args.len() + 2);
    if is_rust_lld(linker) {
        // rust-lld is a generic driver; this makes it MSVC-compatible lld-link.
        forwarded.extend(["-flavor".into(), "link".into()]);
    }
    for arg in args {
        let file = arg
            .to_str()
            .and_then(|a| a.strip_prefix('@'))
            .map(Path::new);
        match file.map(quoted_copy).transpose()? {
            Some(Some(copy)) => forwarded.push(OsString::from(format!("@{}", copy.display()))),
            // Not dx's file: rustc's own response files are UTF-16 and already quoted.
            _ => forwarded.push(arg),
        }
    }
    Ok(forwarded)
}

fn is_rust_lld(linker: &Path) -> bool {
    linker
        .file_stem()
        .is_some_and(|stem| stem.eq_ignore_ascii_case("rust-lld"))
}

/// A quoted copy of a dx response file (plain UTF-8, one argument per line),
/// or `None` for any other response file, which is passed on unchanged.
fn quoted_copy(file: &Path) -> Result<Option<PathBuf>, String> {
    let bytes =
        std::fs::read(file).map_err(|e| format!("could not read {}: {e}", file.display()))?;
    if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        return Ok(None);
    }
    let Ok(text) = String::from_utf8(bytes) else {
        return Ok(None);
    };
    let mut copy = file.as_os_str().to_owned();
    copy.push(".quoted");
    let copy = PathBuf::from(copy);
    std::fs::write(&copy, quote_lines(&text))
        .map_err(|e| format!("could not write {}: {e}", copy.display()))?;
    Ok(Some(copy))
}

/// Quotes every line that contains whitespace, using the MSVC rules:
/// backslashes are literal except right before a quote, so trailing
/// backslashes are doubled before the closing quote.
fn quote_lines(text: &str) -> String {
    text.lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            if line.contains(char::is_whitespace) && !line.starts_with('"') {
                let trailing = line.len() - line.trim_end_matches('\\').len();
                format!("\"{line}{}\"", "\\".repeat(trailing))
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_with_spaces_are_quoted_and_others_are_kept() {
        let text = "/OUT:C:\\a\\world.exe\n/LIBPATH:C:\\Program Files\\VS\\lib\\x64\nfoo.rlib\n";
        assert_eq!(
            quote_lines(text),
            "/OUT:C:\\a\\world.exe\n\"/LIBPATH:C:\\Program Files\\VS\\lib\\x64\"\nfoo.rlib"
        );
    }

    #[test]
    fn a_trailing_backslash_cannot_escape_the_closing_quote() {
        // C:\Program Files\  ->  "C:\Program Files\\"
        assert_eq!(
            quote_lines("C:\\Program Files\\"),
            "\"C:\\Program Files\\\\\""
        );
    }

    #[test]
    fn rust_lld_gets_the_link_flavor_first_and_response_files_are_swapped() {
        let dir = std::env::temp_dir().join(format!("xtask_shim_args_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let rsp = dir.join("args.txt");
        std::fs::write(&rsp, r"/LIBPATH:C:\Program Files\x").unwrap();
        let lld = Path::new("bin").join("rust-lld.exe");
        let args = vec![
            OsString::from("/NOLOGO"),
            OsString::from(format!("@{}", rsp.display())),
        ];
        let out = linker_args(&lld, args).unwrap();
        assert_eq!(out[..3], ["-flavor", "link", "/NOLOGO"].map(OsString::from));
        assert_eq!(out[3], OsString::from(format!("@{}.quoted", rsp.display())));
        // Another linker gets no flavor flag.
        let out = linker_args(&Path::new("bin").join("link.exe"), vec!["/NOLOGO".into()]).unwrap();
        assert_eq!(out, ["/NOLOGO"].map(OsString::from));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn xtask_commands_are_never_mistaken_for_a_link() {
        assert!(looks_like_linker_arg(r"@C:\tmp\args.txt"));
        assert!(looks_like_linker_arg("/NOLOGO"));
        assert!(looks_like_linker_arg("-flavor"));
        assert!(!looks_like_linker_arg("test"));
        assert!(!looks_like_linker_arg("hot"));
    }

    #[test]
    fn rust_lld_is_recognised_by_name() {
        // Built with `join` so the test means the same on every OS.
        let bin = Path::new("toolchain").join("bin");
        assert!(is_rust_lld(&bin.join("rust-lld.exe")));
        assert!(is_rust_lld(&bin.join("RUST-LLD.EXE")));
        assert!(!is_rust_lld(&bin.join("link.exe")));
    }

    #[test]
    fn already_quoted_lines_are_left_alone() {
        assert_eq!(quote_lines("\"C:\\a b\""), "\"C:\\a b\"");
    }

    #[test]
    fn response_files_are_copied_quoted_and_the_original_kept() {
        let dir = std::env::temp_dir().join(format!("xtask_shim_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("args.txt");
        std::fs::write(&file, "/LIBPATH:C:\\Program Files\\x\nmain.o").unwrap();
        let copy = quoted_copy(&file).unwrap().unwrap();
        assert_eq!(
            std::fs::read_to_string(&copy).unwrap(),
            "\"/LIBPATH:C:\\Program Files\\x\"\nmain.o"
        );
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "/LIBPATH:C:\\Program Files\\x\nmain.o"
        );

        // rustc's own UTF-16 response files are not touched.
        let utf16 = dir.join("rustc.rsp");
        let mut bytes = vec![0xff, 0xfe];
        bytes.extend("\"a b\"".encode_utf16().flat_map(u16::to_le_bytes));
        std::fs::write(&utf16, bytes).unwrap();
        assert_eq!(quoted_copy(&utf16).unwrap(), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
