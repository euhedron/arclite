use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// The exit-code contract, shown under `arc --help` — load-bearing for hooks/CI/agents that branch
/// on status. The gate code is formatted in from its one definition, [`crate::synth::GATE_BLOCKED_EXIT`].
fn exit_codes_help() -> String {
    format!(
        "Exit codes:\n  0  success\n  1  error\n  {}  blocked — an opt-in --fail-on-findings gate found findings",
        crate::synth::GATE_BLOCKED_EXIT
    )
}

/// Top-level arclite command-line interface.
#[derive(Debug, Parser)]
#[command(
    name = "arc",
    version,
    about = env!("CARGO_PKG_DESCRIPTION"),
    after_help = exit_codes_help(),
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,

    #[command(subcommand)]
    pub command: Command,
}

/// Options available to every subcommand.
#[derive(Debug, Args)]
pub struct GlobalArgs {
    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long, global = true)]
    pub json: bool,
}

/// The set of arclite subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Report runtime, environment, and available tooling.
    Doctor(DoctorArgs),
    /// Walk a repository and report structured facts about it.
    Inspect(InspectArgs),
    /// Scaffold a repository's `.arc` config (and, with --hook, a pre-push gate).
    Init(InitArgs),
    /// Report the runs currently in flight (the active-run registry).
    Status(StatusArgs),
    /// List the rules in play: the active ruleset, its sources, and each rule's provenance.
    Rules(RulesArgs),
    /// Get, set, or list arclite settings (`config list` shows the keys).
    Config(ConfigArgs),
    /// Show the run history, or one run's full result with `<id>` (the completed-run log).
    Log(LogArgs),
    /// Aggregate run stats from the log — runs, blocks, tokens, and cost over the past hour, day,
    /// week, and all time, plus per-command totals.
    Usage(UsageArgs),
    /// Generate a shell completion script for `arc` (write it where your shell loads completions).
    Completions(CompletionsArgs),
    /// Synthesize a brief assessment of a repository.
    Summarize(SynthArgs),
    /// Suggest where attention is best spent in a repository.
    Suggest(SynthArgs),
    /// Extract reusable rules (standards, anti-patterns, principles) from a repository.
    Extract(SynthArgs),
    /// Audit a repository against selected rules, reporting only violations.
    Audit(SynthArgs),
    /// Critically review a repo for quality defects.
    Critique(SynthArgs),
    /// Propose radical, drastic ways a repository could evolve — overhauls and reimaginings.
    Evolve(SynthArgs),
}

#[derive(Debug, Args)]
pub struct DoctorArgs {}

#[derive(Debug, Args)]
pub struct StatusArgs {}

/// Arguments for `arc rules`.
#[derive(Debug, Args)]
pub struct RulesArgs {
    /// Path to the repository or directory (defaults to the current directory).
    #[arg(default_value = ".")]
    pub path: PathBuf,
    /// Use a named ruleset from settings (else the configured default).
    #[arg(long, value_name = "ID")]
    pub ruleset: Option<String>,
    /// An ad-hoc rule directory or file, overriding the ruleset.
    #[arg(long, value_name = "PATH")]
    pub rules: Option<PathBuf>,
}

/// Arguments for `arc config`.
#[derive(Debug, Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub action: ConfigAction,
}

/// What `arc config` does.
#[derive(Debug, Subcommand)]
pub enum ConfigAction {
    /// Show all resolved settings and the active settings layers.
    List,
    /// Print one setting's resolved value (e.g. `defaults.model`).
    Get {
        /// The setting key — `arc config list` shows the known keys.
        key: String,
    },
    /// Set a setting in a layer — the project's `.arc` by default, `--user` for `~/.arc`.
    Set {
        /// The setting key — `arc config list` shows the known keys.
        key: String,
        /// The value (validated and typed per key).
        value: String,
        /// Write the user layer (`~/.arc/settings.json`) instead of the project's.
        #[arg(long)]
        user: bool,
    },
}

/// Arguments for `arc log`.
#[derive(Debug, Args)]
pub struct LogArgs {
    /// A run id — or unique id prefix — to show in full; omit to list recent runs.
    pub id: Option<String>,
    /// Show the newest run (after any filters) in full, e.g. `arc log --last --command audit`.
    #[arg(long, conflicts_with = "id")]
    pub last: bool,
    /// Only runs of this command (e.g. `audit`).
    #[arg(long, value_name = "CMD")]
    pub command: Option<String>,
    /// Only runs whose repo path contains this (case-insensitive).
    #[arg(long, value_name = "TEXT")]
    pub repo: Option<String>,
    /// Only runs where the gate blocked.
    #[arg(long)]
    pub blocked: bool,
    /// List all matching runs, not just the most recent.
    #[arg(long)]
    pub all: bool,
}

#[derive(Debug, Args)]
pub struct UsageArgs {}

/// Arguments for `arc completions`.
#[derive(Debug, Args)]
pub struct CompletionsArgs {
    /// The shell to generate for.
    pub shell: clap_complete::Shell,
}

/// Arguments for `arc inspect`.
#[derive(Debug, Args)]
pub struct InspectArgs {
    /// Path to the repository or directory to inspect (defaults to the current directory).
    #[arg(default_value = ".")]
    pub path: PathBuf,
}

/// Arguments for `arc init`.
#[derive(Debug, Args)]
pub struct InitArgs {
    /// Path to the repository to scaffold (defaults to the current directory).
    #[arg(default_value = ".")]
    pub path: PathBuf,
    /// Also scaffold a pre-push gate hook and activate it via `core.hooksPath` (opt-in: spends AI on push).
    #[arg(long)]
    pub hook: bool,
}

/// Shared arguments for the synthesis commands.
#[derive(Debug, Args)]
pub struct SynthArgs {
    /// Path to the repository or directory (defaults to the current directory).
    #[arg(default_value = ".")]
    pub path: PathBuf,
    /// Model id to use (a claude or codex model, matching `--backend`). Omit for the backend's default.
    #[arg(long)]
    pub model: Option<String>,
    /// Synthesis backend: `claude` (default) or `codex`. Codex reports token usage but no dollar cost,
    /// and `--max-budget-usd` is claude-only. Overrides the configured `defaults.backend`.
    #[arg(long, value_name = "NAME")]
    pub backend: Option<String>,
    /// Build and show the prompt + a token/cost estimate WITHOUT calling the model (zero spend).
    #[arg(long)]
    pub dry_run: bool,
    /// Hard per-run cost cap in dollars, enforced by the CLI between turns: the run stops as an
    /// error once its spend crosses the cap (the call in flight completes, so the total can
    /// overshoot). Overrides the configured `defaults.max_budget_usd`; unset = no cap.
    #[arg(long, value_name = "USD")]
    pub max_budget_usd: Option<f64>,
    /// Allow a Claude tool during synthesis (repeatable). Default: none.
    #[arg(long = "allow-tool", value_name = "TOOL")]
    pub allow_tool: Vec<String>,
    /// A rule directory or `.md` file to weigh the synthesis against (anti-patterns, standards,
    /// principles, …). Ad-hoc override that takes precedence over `--ruleset` and the default.
    #[arg(long, value_name = "PATH")]
    pub rules: Option<PathBuf>,
    /// Use a named ruleset from `.arc/settings.json` (composing its sources). Overrides the
    /// configured `defaults.ruleset`; `--rules <DIR>` overrides both.
    #[arg(long, value_name = "ID")]
    pub ruleset: Option<String>,
    /// Include a file or directory in the context (repeatable). Directories are walked
    /// gitignore-aware — e.g. `--include src`. Files are read in full by default.
    #[arg(long, value_name = "PATH")]
    pub include: Vec<PathBuf>,
    /// Optional compression: cap each included file (and the README/manifests) at N chars.
    /// Default: none — files are read in full; capping is never automatic, and any cut is
    /// surfaced in the run's sources.
    #[arg(long, value_name = "N")]
    pub max_file_chars: Option<usize>,
    /// Add files changed in git (staged, unstaged, or untracked) to the context, alongside any
    /// `--include`. Works with any command. Default: off.
    #[arg(long)]
    pub changed: bool,
    /// Also write the synthesis to `<DIR>/<command>.md` — a self-describing generated doc (the
    /// directory is created if needed). Stdout output is unchanged; `--dry-run` writes nothing.
    #[arg(long, value_name = "DIR")]
    pub output: Option<PathBuf>,
    /// Load the Claude CLI's ambient memory (your user/project `CLAUDE.md` + auto-memory) into the
    /// synthesis. Default: off — arclite isolates the run so the reported context is authoritative
    /// and reproducible across machines. Enable to deliberately apply your own CLAUDE.md standards.
    #[arg(long)]
    pub ambient_memory: bool,
    /// Emit the command's structured output instead of prose, where the command defines one: a
    /// schema-validated `results` array plus a required `note` — the run's overall read, so an
    /// empty list is a judged outcome, not silence. Commands without a structured mode reject the
    /// flag. Compose with `--json` for machine consumption.
    #[arg(long)]
    pub structured: bool,
    /// Gate on the command's results: exit non-zero if its structured `results` array is
    /// non-empty — for enforcement in git hooks/CI, where a hook blocks on exit status alone. Opt-in;
    /// implies `--structured`; rejected by commands with no structured mode (e.g. `summarize`). Off by
    /// default — no command gates unless asked.
    #[arg(long)]
    pub fail_on_findings: bool,
    /// Order the results from most to least significant (priority/severity/relevance). Applies to
    /// any command; off by default (results come back unordered).
    #[arg(long)]
    pub ranked: bool,
    /// Label each result with a `kind` (its category of finding). Like `--ranked`, it shapes the
    /// output in any mode — guiding prose, or adding a `kind` field to structured results. A command
    /// may suggest a vocabulary; the model may use its own label when none fit. Off by default.
    #[arg(long)]
    pub kinds: bool,
    /// Run the synthesis N times concurrently. Structured `results` are unioned (only byte-identical
    /// items collapse); prose outputs are concatenated as per-run sections. Default: 1; bounded to a
    /// small maximum — a larger value is rejected, never silently capped. A per-run `--max-budget-usd`
    /// applies to each, so N runs can spend up to N× it.
    #[arg(long, default_value_t = 1)]
    pub runs: usize,
}
