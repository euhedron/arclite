//! arclite — an agent-first CLI for cross-repo code intelligence and auditing.

use std::process::ExitCode;

use clap::Parser;

mod ai;
mod cli;
mod commands;
mod http;
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

/// A repo's open-findings ledger directory, `<repo>/.arc/findings/open` — single-sourced (like
/// [`ARC_DIR`]) so the promote writer and the `--findings` reader share one layout and can't drift.
pub(crate) fn findings_open_dir(repo_root: &std::path::Path) -> std::path::PathBuf {
    repo_root.join(ARC_DIR).join("findings").join("open")
}

/// The repo's resolved findings dir (`.arc/findings/resolved`) — where `retire` moves a finding once a
/// verify run judges it `resolved`: the open ledger's counterpart, the other end of the lifecycle.
pub(crate) fn findings_resolved_dir(repo_root: &std::path::Path) -> std::path::PathBuf {
    repo_root.join(ARC_DIR).join("findings").join("resolved")
}

/// The ledger path for an entry id: `<dir>/<id>.md`. One definition shared by the dry-run preview and
/// the real write in both `promote` and `retire`, so the entry-name convention has a single home.
pub(crate) fn findings_entry_path(dir: &std::path::Path, id: &str) -> std::path::PathBuf {
    dir.join(format!("{id}.md"))
}

/// The candidate entry paths for `stem`, in claim order (`stem.md`, `stem-1.md`, …) — the one
/// sequence both the real claim and the dry-run preview walk, so what a preview names and what a
/// run would claim can't drift (preview-must-share-execution-path).
fn findings_entry_candidates<'a>(
    dir: &'a std::path::Path,
    stem: &'a str,
) -> impl Iterator<Item = std::path::PathBuf> + 'a {
    (0u32..).map(move |n| {
        let name = if n == 0 {
            stem.to_owned()
        } else {
            format!("{stem}-{n}")
        };
        findings_entry_path(dir, &name)
    })
}

/// The path a promote/retire *would* claim for `stem` right now: the first candidate neither on
/// disk nor in `reserved` — the batch's own earlier predictions, which a dry run must honor the way
/// the real run's atomic claims would (two colliding slugs preview `x.md` and `x-1.md`, not `x.md`
/// twice). The chosen path is added to `reserved`. Probes use `try_exists` semantics: an unreadable
/// candidate is a real error, never read as "free" (which would preview a name the real claim would
/// refuse). Still indicative under concurrency — an outside writer can take a name between preview
/// and run.
pub(crate) fn preview_findings_entry(
    dir: &std::path::Path,
    stem: &str,
    reserved: &mut std::collections::HashSet<std::path::PathBuf>,
) -> std::io::Result<std::path::PathBuf> {
    for path in findings_entry_candidates(dir, stem) {
        if !reserved.contains(&path) && !path.try_exists()? {
            reserved.insert(path.clone());
            return Ok(path);
        }
    }
    unreachable!("the candidate sequence is unbounded, so the loop returns")
}

/// Claim a collision-free `<stem>[-n].md` under `dir`, returning the path and an open handle to write.
/// `create_new` fails if the name exists, so a concurrent writer bumps a numeric suffix rather than
/// clobbering — the concurrency-safe ledger-entry claim shared by `promote` (a new finding) and `retire`
/// (a moved, resolved one), single-sourced so the naming convention can't drift between them. The caller
/// writes its own body (the two differ in content), keeping only the name-claim here.
pub(crate) fn claim_findings_entry(
    dir: &std::path::Path,
    stem: &str,
) -> std::io::Result<(std::path::PathBuf, std::fs::File)> {
    std::fs::create_dir_all(dir)?;
    for path in findings_entry_candidates(dir, stem) {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!("the candidate sequence is unbounded, so the loop returns")
}

/// `git config --get <key>` in `dir`: `Ok(Some(value))` if set, `Ok(None)` if unset (git exits 1 —
/// benign), `Err` on a real config failure (exit >1, e.g. a corrupt or locked config) — never
/// collapsing unset with failure. Single-sourced so `init` and `doctor` read git config the same way.
pub(crate) fn git_config_get(dir: &std::path::Path, key: &str) -> anyhow::Result<Option<String>> {
    let output = crate::ai::command("git")?
        .current_dir(dir)
        .args(["config", "--get", key])
        .output()
        .map_err(|e| anyhow::anyhow!("could not run git to read {key}: {e}"))?;
    match output.status.code() {
        Some(0) => Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        )),
        Some(1) => Ok(None),
        _ => anyhow::bail!(
            "git config --get {key} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    }
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

pub(crate) fn join_or(items: &[String], empty: &str) -> String {
    if items.is_empty() {
        empty.to_owned()
    } else {
        items.join(", ")
    }
}

/// The home-directory prefix [`display_path`] abbreviates, resolved once at startup ([`run`] warms
/// it before any command) so the display helper — called from render projections — only ever reads
/// a fixed value, never probes the environment mid-format.
static DISPLAY_HOME: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

fn display_home() -> &'static Option<String> {
    DISPLAY_HOME.get_or_init(|| dirs::home_dir().and_then(|h| h.to_str().map(str::to_owned)))
}

/// Abbreviate a leading home-directory prefix to `~` for *display* (e.g. `C:\Users\x\proj` → `~\proj`);
/// paths outside home are returned unchanged. Cosmetic only — applied where a path is shown to a
/// person, never in error messages (which keep the exact path) nor where a value must round-trip
/// (the stored record stays canonical). Reads the startup-resolved [`DISPLAY_HOME`], probing nothing.
pub(crate) fn display_path(path: &str) -> String {
    if let Some(home) = display_home()
        && let Some(rest) = path.strip_prefix(home)
        && (rest.is_empty() || rest.starts_with(['/', '\\']))
    {
        return format!("~{rest}");
    }
    path.to_owned()
}

pub(crate) fn labeled_row(label: &str, value: &str, width: usize) -> String {
    format!("{label:<width$}{value}")
}

/// Parse arguments, dispatch to the selected command, and map the result to a process exit code:
/// `SUCCESS`, the gate's distinct block code, or `FAILURE` with the error on stderr.
#[must_use]
pub fn run() -> ExitCode {
    // Resolve the display home-prefix once, up front — render paths read it, they never probe.
    let _ = display_home();
    let cli = Cli::parse();

    // Deterministic commands always succeed-or-error (mapped to SUCCESS); the synthesis commands
    // return their own ExitCode so an opt-in gate (`--fail-on-findings`) surfaces as a distinct
    // non-zero code without being an error.
    let result = match &cli.command {
        Command::Doctor(args) => {
            commands::doctor::run(args, &cli.global).map(|()| ExitCode::SUCCESS)
        }
        Command::Update(args) => {
            commands::update::run(args, &cli.global).map(|()| ExitCode::SUCCESS)
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
        Command::Models(args) => {
            commands::models::run(args, &cli.global).map(|()| ExitCode::SUCCESS)
        }
        Command::Config(args) => {
            commands::config::run(args, &cli.global).map(|()| ExitCode::SUCCESS)
        }
        Command::Log(args) => commands::log::run(args, &cli.global).map(|()| ExitCode::SUCCESS),
        Command::Usage(args) => commands::usage::run(args, &cli.global).map(|()| ExitCode::SUCCESS),
        Command::Promote(args) => {
            commands::promote::run(args, &cli.global).map(|()| ExitCode::SUCCESS)
        }
        Command::Retire(args) => {
            commands::retire::run(args, &cli.global).map(|()| ExitCode::SUCCESS)
        }
        // `completions` emits a shell script, not JSON — reject `--json` rather than accept and ignore
        // it (an explicit option silently dropped is worse than a silent default).
        Command::Completions(_) if cli.global.json => Err(anyhow::anyhow!(
            "`--json` has no meaning for `arc completions` (it emits a shell completion script)"
        )),
        Command::Completions(args) => {
            // The binary name is single-sourced in `cli::binary_name`; the command itself (which
            // `generate` needs by &mut) is built here.
            let mut command = <Cli as clap::CommandFactory>::command();
            clap_complete::generate(
                args.shell,
                &mut command,
                crate::cli::binary_name(),
                &mut std::io::stdout(),
            );
            Ok(ExitCode::SUCCESS)
        }
        Command::Run(args) => {
            // The verb registry owns the enum→verb mapping (verbs::resolve, beside verbs::ALL);
            // this call site just drives the resolved row.
            let (verb, a) = commands::verbs::resolve(&args.verb);
            verb.run(a, &cli.global)
        }
    };

    match result {
        Ok(code) => code,
        Err(err) => {
            eprintln!("arclite: {err:#}");
            ExitCode::FAILURE
        }
    }
}
