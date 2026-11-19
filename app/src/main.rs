//! The World executable. Thin on purpose: parse, configure, assemble, run.

mod cli;
mod settings;
mod startup;

fn main() -> std::process::ExitCode {
    startup::run()
}
