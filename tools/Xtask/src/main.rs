//! `cargo xtask`: the single entry point for tests, goldens, shader codegen,
//! benchmarks, checks, fuzzing, mutation testing and running the game with
//! live reloading.

mod bench;
mod check;
mod codegen;
mod golden;
mod hot;
mod process;
mod testing;

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use testing::TestProfile;

#[derive(Parser)]
#[command(name = "xtask", about = "World automation")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run tests. `fast` (default) skips GPU tests, `gpu` runs only them,
    /// `full` runs everything at the plan's one-million-edit scale, `all`
    /// runs everything at fast scale.
    Test {
        #[arg(value_enum, default_value_t = TestProfile::Fast)]
        profile: TestProfile,
        /// Extra arguments passed to the test runner, after `--`.
        #[arg(last = true)]
        extra: Vec<String>,
    },
    /// Check golden files, or rewrite them with --update after reviewing the output.
    Golden {
        #[arg(long)]
        update: bool,
    },
    /// Generate the WESL shader constants from World_Render's Rust constants
    /// and show what changed. --check writes nothing and fails if they are stale.
    Codegen {
        #[arg(long)]
        check: bool,
    },
    /// Run benchmarks. --save stores a baseline; --check compares against one
    /// and fails when anything got slower than the threshold.
    Bench {
        #[arg(long, value_name = "NAME", conflicts_with = "check")]
        save: Option<String>,
        #[arg(long, value_name = "NAME")]
        check: Option<String>,
        /// Allowed slowdown in percent before --check fails: wall time, or
        /// instruction count with --instructions.
        #[arg(long, default_value_t = 10.0)]
        threshold: f64,
        /// Count instructions with gungraun under Valgrind instead of timing
        /// with Criterion (Linux only; needs valgrind and gungraun-runner).
        #[arg(long)]
        instructions: bool,
    },
    /// Formatting, clippy with warnings denied, then the fast tests.
    Check,
    /// Line coverage report (HTML) with cargo-llvm-cov.
    Coverage,
    /// Run the unsafe-capable crates under Miri (needs the nightly toolchain).
    Miri,
    /// Fuzz World_Core's decoder with cargo-fuzz (nightly): every target, or
    /// the one named, for --time seconds each. Crashes land in
    /// crates/Foundation/World_Core/fuzz/artifacts.
    Fuzz {
        #[arg(value_parser = clap::builder::PossibleValuesParser::new(check::fuzz::TARGETS))]
        target: Option<String>,
        /// Seconds to fuzz each target.
        #[arg(long, default_value_t = 300)]
        time: u64,
    },
    /// Mutation testing of World_Core with cargo-mutants (see
    /// .cargo/mutants.toml). Arguments after `--` go to cargo-mutants.
    Mutants {
        #[arg(last = true)]
        extra: Vec<String>,
    },
    /// Run the game with asset, config and shader hot reload and the developer
    /// panels (F1 inspector, F2 configs, F5 re-run setup). Arguments after `--`
    /// go to the game.
    Dev {
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Run the game under `dx serve --hot-patch`: everything `dev` reloads,
    /// plus Rust edits to systems while the game runs. --smoke runs the
    /// automated end-to-end hot-patch test instead.
    Hot {
        #[arg(long)]
        smoke: bool,
        #[arg(last = true)]
        args: Vec<String>,
    },
}

fn main() -> ExitCode {
    // Started by dx as the linker during `xtask hot`: act as the link shim.
    if let Some(linker) = hot::link_shim::invoked_as_shim() {
        return hot::link_shim::run(&linker);
    }
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Test { profile, extra } => testing::run(profile, &extra),
        Command::Golden { update } => golden::run(update),
        Command::Codegen { check } => codegen::run(check),
        Command::Bench {
            save,
            check,
            threshold,
            instructions,
        } => bench::run(save, check, threshold, instructions),
        Command::Check => check::run(),
        Command::Coverage => check::coverage(),
        Command::Miri => check::miri(),
        Command::Fuzz { target, time } => check::fuzz::run(target.as_deref(), time),
        Command::Mutants { extra } => check::mutants::run(&extra),
        Command::Dev { args } => hot::dev(&args),
        Command::Hot { smoke, args } => hot::hot(&args, smoke),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("\nxtask: {message}");
            ExitCode::FAILURE
        }
    }
}
