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

/// Read a file's text, with a missing file as `None` — the one statement of the "absent is benign,
/// any other IO failure is a real error" distinction, shared by every optional-file read so a
/// permission/corruption failure can't masquerade as "nothing there yet". Each caller decides what
/// absence means.
pub(crate) fn read_optional(path: &std::path::Path) -> std::io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Resolve a user-supplied path against `base`: a leading `~/` (or `~\`) expands to the home
/// directory, an absolute path stands as-is, and a relative one joins `base`. The one statement of
/// this resolution — ruleset sources (relative to their settings file) and `--include` paths
/// (relative to the repo root) share it, so the two can't drift.
pub(crate) fn resolve_path(base: &std::path::Path, raw: &std::path::Path) -> std::path::PathBuf {
    if let Some(rest) = raw
        .to_str()
        .and_then(|s| s.strip_prefix("~/").or_else(|| s.strip_prefix("~\\")))
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        base.join(raw)
    }
}

/// List a directory, with a missing directory as `None` — [`read_optional`]'s absent-vs-failed
/// distinction for directories, shared by the run-registry and result-store listings.
pub(crate) fn read_dir_optional(dir: &std::path::Path) -> std::io::Result<Option<std::fs::ReadDir>> {
    match std::fs::read_dir(dir) {
        Ok(entries) => Ok(Some(entries)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Parse arguments, dispatch to the selected command, and map the result to a process exit code:
/// `SUCCESS`, the gate's distinct block code, or `FAILURE` with the error on stderr.
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
        Command::Log(args) => commands::log::run(args, &cli.global).map(|()| ExitCode::SUCCESS),
        Command::Usage(args) => {
            commands::usage::run(args, &cli.global).map(|()| ExitCode::SUCCESS)
        }
        Command::Completions(args) => {
            clap_complete::generate(
                args.shell,
                &mut <Cli as clap::CommandFactory>::command(),
                "arc",
                &mut std::io::stdout(),
            );
            Ok(ExitCode::SUCCESS)
        }
        Command::Summarize(args) => commands::summarize::run(args, &cli.global),
        Command::Suggest(args) => commands::suggest::run(args, &cli.global),
        Command::Extract(args) => commands::extract::run(args, &cli.global),
        Command::Audit(args) => commands::audit::run(args, &cli.global),
        Command::Critique(args) => commands::critique::run(args, &cli.global),
        Command::Evolve(args) => commands::evolve::run(args, &cli.global),
    };

    match result {
        Ok(code) => code,
        Err(err) => {
            eprintln!("arclite: {err:#}");
            ExitCode::FAILURE
        }
    }
}
