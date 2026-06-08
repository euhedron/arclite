//! arclite — an agent-first CLI for cross-repo code intelligence and auditing.

use std::process::ExitCode;

use clap::Parser;

mod ai;
mod cli;
mod commands;
mod log;
mod output;
mod rules;
mod settings;
mod synth;
mod walk;

use cli::{Cli, Command};

/// arclite's per-scope config/data directory (`~/.arc`, `<repo>/.arc`): settings, rules, and logs.
pub(crate) const ARC_DIR: &str = ".arc";

/// Parse arguments, dispatch to the selected command, and map the result to a
/// process exit code (`SUCCESS`, or `FAILURE` with the error on stderr).
///
/// Predictable exit codes keep arclite scriptable by both agents and humans.
#[must_use]
pub fn run() -> ExitCode {
    let cli = Cli::parse();

    // Deterministic commands always succeed-or-error (mapped to SUCCESS); the synthesis commands
    // return their own ExitCode so an opt-in gate (`--fail-on-findings`) surfaces as a distinct
    // non-zero code without being an error.
    let result = match &cli.command {
        Command::Doctor(args) => commands::doctor::run(args, &cli.global).map(|()| ExitCode::SUCCESS),
        Command::Inspect(args) => {
            commands::inspect::run(args, &cli.global).map(|()| ExitCode::SUCCESS)
        }
        Command::Summarize(args) => commands::summarize::run(args, &cli.global),
        Command::Suggest(args) => commands::suggest::run(args, &cli.global),
        Command::Extract(args) => commands::extract::run(args, &cli.global),
        Command::Audit(args) => commands::audit::run(args, &cli.global),
        Command::Critique(args) => commands::critique::run(args, &cli.global),
    };

    match result {
        Ok(code) => code,
        Err(err) => {
            eprintln!("arclite: {err:#}");
            ExitCode::FAILURE
        }
    }
}
