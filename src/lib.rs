//! arclite — an agent-first CLI for cross-repo code intelligence and auditing.

use std::process::ExitCode;

use clap::Parser;

mod cli;
mod commands;
mod output;

use cli::{Cli, Command};

/// Parse arguments, dispatch to the selected command, and map the result to a
/// process exit code (`SUCCESS`, or `FAILURE` with the error on stderr).
///
/// Predictable exit codes keep arclite scriptable by both agents and humans.
#[must_use]
pub fn run() -> ExitCode {
    let cli = Cli::parse();

    let result = match &cli.command {
        Command::Doctor(args) => commands::doctor::run(args, &cli.global),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("arclite: {err:#}");
            ExitCode::FAILURE
        }
    }
}
