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

/// The real linker, when this process was started as the linker shim.
pub fn invoked_as_shim() -> Option<PathBuf> {
    std::env::var_os(REAL_LINKER_ENV).map(PathBuf::from)
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
    let mut forwarded = Vec::with_capacity(args.len());
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
    // Output passes straight through, so dx sees the linker's own messages.
    let mut command = Command::new(linker);
    if is_rust_lld(linker) {
        // rust-lld is a generic driver; this makes it MSVC-compatible lld-link.
        command.args(["-flavor", "link"]);
    }
    let status = command
        .args(&forwarded)
        .status()
        .map_err(|e| format!("could not run {}: {e}", linker.display()))?;
    Ok(match status.code() {
        Some(0) => ExitCode::SUCCESS,
        Some(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        None => ExitCode::FAILURE,
    })
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
    fn rust_lld_is_recognised_by_name() {
        assert!(is_rust_lld(Path::new(r"C:\rust\bin\rust-lld.exe")));
        assert!(!is_rust_lld(Path::new(r"C:\VS\link.exe")));
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
