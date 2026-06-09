//! arclite — an agent-first CLI for cross-repo code intelligence and auditing.

use std::process::ExitCode;

use clap::Parser;

mod ai;
mod cli;
mod commands;
mod log;
mod output;
mod rules;
mod runs;
mod settings;
mod synth;
mod walk;

use cli::{Cli, Command};

/// arclite's per-scope config/data directory (`~/.arc`, `<repo>/.arc`): settings, rules, and logs.
pub(crate) const ARC_DIR: &str = ".arc";

/// The user-level arclite directory, `~/.arc` (`None` if the home directory is unknown) — the single
/// source for the home-relative base that the run log, the run registry, and user settings build on.
pub(crate) fn arc_home() -> Option<std::path::PathBuf> {
    Some(dirs::home_dir()?.join(ARC_DIR))
}

/// The settings filename inside an `.arc` directory — single-sourced (like [`ARC_DIR`]) so a rename
/// can't rot across the user/project loaders, `config`, and `init`.
pub(crate) const SETTINGS_FILE: &str = "settings.json";

/// Parse arguments, dispatch to the selected command, and map the result to a process exit code:
/// `SUCCESS`, the gate's block code (2), or `FAILURE` with the error on stderr.
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
        Command::Init(args) => commands::init::run(args, &cli.global).map(|()| ExitCode::SUCCESS),
        Command::Status(args) => {
            commands::status::run(args, &cli.global).map(|()| ExitCode::SUCCESS)
        }
        Command::Rules(args) => commands::rules::run(args, &cli.global).map(|()| ExitCode::SUCCESS),
        Command::Config(args) => {
            commands::config::run(args, &cli.global).map(|()| ExitCode::SUCCESS)
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
