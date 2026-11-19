//! The World executable. Thin on purpose: parse, configure, assemble, run.

// Bevy's dynamic linking (the `dev` feature) breaks hot-patching on Windows
// (decision 0002). `cargo xtask hot` builds with `--no-default-features`.
#[cfg(all(feature = "dev", feature = "hotpatch"))]
compile_error!(
    "the `dev` and `hotpatch` features cannot be combined: build with \
     `--no-default-features --features hotpatch`, or run `cargo xtask hot`"
);

mod cli;
mod settings;
mod startup;

fn main() -> std::process::ExitCode {
    startup::run()
}
