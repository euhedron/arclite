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

use cli::{Cli, Command, RunVerb};

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

/// Map a fallible read to optional: a `NotFound` error becomes `Ok(None)` (absent is benign), any
/// other error propagates. The one statement of the "absent vs present-but-failed" distinction, so a
/// permission/corruption failure can't masquerade as "nothing there yet"; each caller decides what
/// absence means.
fn optional<T>(result: std::io::Result<T>) -> std::io::Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Read a file's text, with a missing file as `Ok(None)` (via [`optional`]).
pub(crate) fn read_optional(path: &std::path::Path) -> std::io::Result<Option<String>> {
    optional(std::fs::read_to_string(path))
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

/// List a directory, with a missing directory as `Ok(None)` (via [`optional`]).
pub(crate) fn read_dir_optional(
    dir: &std::path::Path,
) -> std::io::Result<Option<std::fs::ReadDir>> {
    optional(std::fs::read_dir(dir))
}

/// Whether `path` is an existing directory: absent or a non-directory → `Ok(false)`, a present
/// directory → `Ok(true)`, a permission/I-O failure → a real `Err` (via [`optional`], so an
/// unreadable path can't masquerade as absent). The dir-aware analogue of `std::fs::try_exists`.
pub(crate) fn try_is_dir(path: &std::path::Path) -> std::io::Result<bool> {
    Ok(optional(std::fs::metadata(path))?.is_some_and(|m| m.is_dir()))
}

/// Render `items` as a comma-joined string, or `empty` when there are none — the "one-or-more, else a
/// placeholder" shape shared by settings-layer lines ([`settings::NO_LAYERS`]) and inspect's manifest
/// list (`(none)`), single-sourced so the empty-vs-joined branch isn't re-written at each call site.
pub(crate) fn join_or(items: &[String], empty: &str) -> String {
    if items.is_empty() {
        empty.to_owned()
    } else {
        items.join(", ")
    }
}

/// Abbreviate a leading home-directory prefix to `~` for *display* (e.g. `C:\Users\x\proj` → `~\proj`);
/// paths outside home are returned unchanged. Cosmetic only — applied where a path is shown to a
/// person (the masthead, the context sources, `doctor`, the run reports), never in error messages
/// (which keep the exact path) nor where a value must round-trip (the stored record stays canonical).
pub(crate) fn display_path(path: &str) -> String {
    if let Some(home) = dirs::home_dir().and_then(|h| h.to_str().map(str::to_owned))
        && let Some(rest) = path.strip_prefix(&home)
        && (rest.is_empty() || rest.starts_with(['/', '\\']))
    {
        return format!("~{rest}");
    }
    path.to_owned()
}

/// A label left-padded to `width`, then its value — the single statement of the aligned
/// `label   value` row that `doctor` and `inspect` print, so neither hand-counts whitespace into a
/// format literal (which a renamed label would silently misalign — the anti-pattern the audit flags).
pub(crate) fn labeled_row(label: &str, value: &str, width: usize) -> String {
    format!("{label:<width$}{value}")
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
        Command::Doctor(args) => {
            commands::doctor::run(args, &cli.global).map(|()| ExitCode::SUCCESS)
        }
        Command::Inspect(args) => {
            commands::inspect::run(args, &cli.global).map(|()| ExitCode::SUCCESS)
        }
        Command::Init(args) => commands::init::run(args, &cli.global).map(|()| ExitCode::SUCCESS),
        Command::Status(args) => {
            commands::status::run(args, &cli.global).map(|()| ExitCode::SUCCESS)
        }
        Command::Tui(args) => commands::tui::run(args, &cli.global).map(|()| ExitCode::SUCCESS),
        Command::Rules(args) => commands::rules::run(args, &cli.global).map(|()| ExitCode::SUCCESS),
        Command::Config(args) => {
            commands::config::run(args, &cli.global).map(|()| ExitCode::SUCCESS)
        }
        Command::Log(args) => commands::log::run(args, &cli.global).map(|()| ExitCode::SUCCESS),
        Command::Usage(args) => commands::usage::run(args, &cli.global).map(|()| ExitCode::SUCCESS),
        Command::Promote(args) => {
            commands::promote::run(args, &cli.global).map(|()| ExitCode::SUCCESS)
        }
        Command::Completions(args) => {
            // Take the binary name from the command itself (cli.rs's `#[command(name)]`) rather than
            // repeating an "arc" literal here — one rename point, not several.
            let mut command = <Cli as clap::CommandFactory>::command();
            let bin_name = command.get_name().to_owned();
            clap_complete::generate(args.shell, &mut command, bin_name, &mut std::io::stdout());
            Ok(ExitCode::SUCCESS)
        }
        Command::Run(args) => match &args.verb {
            RunVerb::Summarize(a) => commands::verbs::SUMMARIZE.run(a, &cli.global),
            RunVerb::Suggest(a) => commands::verbs::SUGGEST.run(a, &cli.global),
            RunVerb::Extract(a) => commands::verbs::EXTRACT.run(a, &cli.global),
            RunVerb::Audit(a) => commands::verbs::AUDIT.run(a, &cli.global),
            RunVerb::Critique(a) => commands::verbs::CRITIQUE.run(a, &cli.global),
            RunVerb::Evolve(a) => commands::verbs::EVOLVE.run(a, &cli.global),
        },
    };

    match result {
        Ok(code) => code,
        Err(err) => {
            eprintln!("arclite: {err:#}");
            ExitCode::FAILURE
        }
    }
}
