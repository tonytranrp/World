//! The generated WESL constants module (decision 0013).
//!
//! `assets/shaders/common/constants.wesl` is rendered from
//! `world_render::internals::shader_constants()` and must match the
//! checked-in file byte for byte. `cargo xtask codegen` runs this test with
//! `UPDATE_CODEGEN=1`, which writes the file instead of comparing.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Set to `1` to write the generated file instead of checking it.
const UPDATE_ENV: &str = "UPDATE_CODEGEN";

/// The generated module, relative to the workspace's `assets` folder.
const GENERATED: &str = "shaders/common/constants.wesl";

const HEADER: &str = "\
// GENERATED FILE, DO NOT EDIT. Regenerate with `cargo xtask codegen`.
//
// The constants the shaders share with World_Render, rendered from
// `world_render::internals::shader_constants()` (decision 0013). To change a
// value, change the Rust constant and rerun the command; the test
// `generated_shader_constants_are_current` fails until this file matches.
// Each group is named after the World_Render module that defines it.
";

fn generated_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../assets")
        .join(GENERATED)
}

/// Bit patterns are written in hex (masks, single bits and anything above
/// 255), counts, shifts and enumerations in decimal.
fn literal(name: &str, value: u32) -> String {
    if name.ends_with("_MASK") || name.ends_with("_BIT") || value > 255 {
        format!("{value:#X}u").replacen("0X", "0x", 1)
    } else {
        format!("{value}u")
    }
}

/// A WGSL identifier the shaders can use as a constant name, in the
/// upper-case style the shaders use for constants.
fn valid_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase())
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// The whole text of the generated module. Byte-stable: the order is the
/// order of `shader_constants()`, and nothing depends on the environment.
fn render() -> String {
    let mut text = String::from(HEADER);
    let mut seen = BTreeSet::new();
    for (module, constants) in world_render::internals::shader_constants() {
        assert!(
            !constants.is_empty(),
            "{module}: an empty constant group would only add a heading"
        );
        write!(text, "\n// {module}\n").unwrap();
        for &(name, value) in constants {
            assert!(
                valid_name(name),
                "{module}: `{name}` is not an upper-case WGSL identifier"
            );
            assert!(
                seen.insert(name),
                "{module}: `{name}` is declared twice; WESL would reject the module"
            );
            writeln!(text, "const {name}: u32 = {};", literal(name, value)).unwrap();
        }
    }
    text
}

#[test]
fn generated_shader_constants_are_current() {
    let path = generated_path();
    let expected = render();
    if std::env::var(UPDATE_ENV).is_ok_and(|v| v == "1") {
        let current = std::fs::read_to_string(&path).ok();
        if current.as_deref() != Some(expected.as_str()) {
            std::fs::write(&path, &expected)
                .unwrap_or_else(|e| panic!("writing {}: {e}", path.display()));
            println!("codegen: wrote {}", path.display());
        }
        return;
    }
    let current = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "reading {}: {e}; run `cargo xtask codegen` to generate it",
            path.display()
        )
    });
    if current != expected {
        let first = current
            .lines()
            .zip(expected.lines())
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| current.lines().count().min(expected.lines().count()));
        panic!(
            "assets/{GENERATED} is out of date with World_Render's shader constants \
             (first difference at line {}).\n  checked in: {:?}\n  generated:  {:?}\n\
             Run `cargo xtask codegen` and commit the result; never edit the file by hand.",
            first + 1,
            current.lines().nth(first).unwrap_or("<end of file>"),
            expected.lines().nth(first).unwrap_or("<end of file>"),
        );
    }
}

#[test]
fn literals_keep_their_written_form() {
    assert_eq!(literal("INFO_KIND_MASK", 3), "0x3u");
    assert_eq!(literal("INFO_EDITED_BIT", 0x40000), "0x40000u");
    assert_eq!(literal("INVALID_REF", u32::MAX), "0xFFFFFFFFu");
    assert_eq!(literal("REQUEST_PRIORITY_MAX", 0xFFF), "0xFFFu");
    assert_eq!(literal("BRICK_REF_SEGMENT_SHIFT", 28), "28u");
    assert_eq!(literal("BLOCK_FLAG_EDITED", 1), "1u");
    assert!(valid_name("HIT_WORDS") && valid_name("A1_B"));
    assert!(!valid_name("hit_words") && !valid_name("1A") && !valid_name("A-B"));
}
