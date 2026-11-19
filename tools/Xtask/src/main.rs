//! `cargo xtask`: the single entry point for tests, goldens, benchmarks and checks.

mod bench;
mod check;
mod golden;
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
    /// Run benchmarks. --save stores a baseline; --check compares against one
    /// and fails when anything got slower than the threshold.
    Bench {
        #[arg(long, value_name = "NAME", conflicts_with = "check")]
        save: Option<String>,
        #[arg(long, value_name = "NAME")]
        check: Option<String>,
        /// Allowed slowdown in percent before --check fails.
        #[arg(long, default_value_t = 10.0)]
        threshold: f64,
    },
    /// Formatting, clippy with warnings denied, then the fast tests.
    Check,
    /// Line coverage report (HTML) with cargo-llvm-cov.
    Coverage,
    /// Run the unsafe-capable crates under Miri (needs the nightly toolchain).
    Miri,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Test { profile, extra } => testing::run(profile, &extra),
        Command::Golden { update } => golden::run(update),
        Command::Bench {
            save,
            check,
            threshold,
        } => bench::run(save, check, threshold),
        Command::Check => check::run(),
        Command::Coverage => check::coverage(),
        Command::Miri => check::miri(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("\nxtask: {message}");
            ExitCode::FAILURE
        }
    }
}
