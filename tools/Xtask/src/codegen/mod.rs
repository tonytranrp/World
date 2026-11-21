//! `xtask codegen`: generate the WESL shader constants from Rust, or check
//! that the checked-in output is current (decision 0013).
//!
//! The constants are defined in World_Render. Xtask does not depend on that
//! crate (it would pull Bevy into every xtask build); it runs the World_Render
//! test that renders the module, the way `xtask golden` runs the golden suites.

use std::process::Command;

use crate::process::{self, cargo};

/// The generated files, relative to the workspace root.
const GENERATED: &[&str] = &["assets/shaders/common/constants.wesl"];

/// Set for the test to write its output instead of comparing.
const UPDATE_ENV: &str = "UPDATE_CODEGEN";

/// The test that renders and checks the generated module.
const TEST_BINARY: &str = "shader_constants";
const TEST_NAME: &str = "generated_shader_constants_are_current";

/// Regenerates the shader constants and shows which generated files changed.
/// With `check`, writes nothing and fails if the checked-in files are stale.
pub fn run(check: bool) -> Result<(), String> {
    let root = process::workspace_root();
    let mut command = cargo();
    // `--workspace` rather than `--package world_render`: a single-package
    // build resolves Bevy's features differently and would rebuild all of
    // Bevy instead of reusing the workspace's test build. Only World_Render
    // has a test target of this name.
    command
        .current_dir(&root)
        .args(["test", "--workspace", "--test", TEST_BINARY])
        .args(["--", "--exact", TEST_NAME]);
    if check {
        // The caller's environment may have it set; a check never writes.
        command.env_remove(UPDATE_ENV);
    } else {
        command.env(UPDATE_ENV, "1");
    }
    run_test(&mut command).map_err(|e| {
        if check {
            format!("{e}\nthe generated shader constants are stale: run `cargo xtask codegen`")
        } else {
            e
        }
    })?;

    if !check {
        println!("\nGenerated files changed by this run (review before committing):");
        let mut status = Command::new("git");
        status
            .current_dir(&root)
            .args(["status", "--short", "--"])
            .args(GENERATED);
        process::run(&mut status)?;
    }
    Ok(())
}

/// Runs the test like `process::run`, and also fails when the filter
/// matched nothing (libtest passes an empty run), so a renamed test cannot
/// turn codegen into a silent no-op.
fn run_test(command: &mut Command) -> Result<(), String> {
    println!("$ {}", process::describe(command));
    let output = command
        .stderr(std::process::Stdio::inherit())
        .output()
        .map_err(|e| format!("could not start cargo: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    print!("{stdout}");
    if !output.status.success() {
        return Err(format!("cargo test failed with {}", output.status));
    }
    if !stdout.contains(&format!("test {TEST_NAME} ... ok")) {
        return Err(format!(
            "the test `{TEST_NAME}` in world_render's `{TEST_BINARY}` did not run; \
             update tools/Xtask/src/codegen if it was renamed"
        ));
    }
    Ok(())
}
