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

/// The binary's invoked name as clap derives it from [`Cli`]'s `#[command(name)]`, single-sourced
/// so callers read one definition and a rename has a single home rather than scattered literals.
pub(crate) fn binary_name() -> String {
    <Cli as clap::CommandFactory>::command()
        .get_name()
        .to_owned()
}

/// Options available to every subcommand.
#[derive(Debug, Args)]
pub struct GlobalArgs {
    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long, global = true)]
    pub json: bool,
}

/// Each synthesis verb's subcommand name + one-line description — single-sourced here so both clap
/// (the `name`/`about` on each verb subcommand below) and the TUI palette (`commands::tui`, which
/// spawns `arc <name>` and shows the description) read the same strings and can't drift. Descriptions
/// are kept terse: they double as palette hints.
/// The `run` group's own subcommand name — shared by clap's `Run` variant, the TUI palette's `run`
/// entry, and the launcher that spawns `arc run <verb>`, so the grouping name can't drift.
pub(crate) const NAME_RUN: &str = "run";
pub(crate) const NAME_SUMMARIZE: &str = "summarize";
pub(crate) const VERB_SUMMARIZE: &str = "Summarize the repository";
pub(crate) const NAME_SUGGEST: &str = "suggest";
pub(crate) const VERB_SUGGEST: &str = "Surface where attention is best spent";
pub(crate) const NAME_EXTRACT: &str = "extract";
pub(crate) const VERB_EXTRACT: &str = "Extract reusable rules from the repo";
pub(crate) const NAME_AUDIT: &str = "audit";
pub(crate) const VERB_AUDIT: &str = "Audit the repo for rule violations";
pub(crate) const NAME_CRITIQUE: &str = "critique";
pub(crate) const VERB_CRITIQUE: &str = "Review the repo for quality defects";
pub(crate) const NAME_VERIFY: &str = "verify";
pub(crate) const VERB_VERIFY: &str = "Re-check open findings against the current code";
pub(crate) const NAME_EVOLVE: &str = "evolve";
pub(crate) const VERB_EVOLVE: &str = "Propose radical ways to evolve the repo";
pub(crate) const NAME_AGGREGATE: &str = "aggregate";
pub(crate) const VERB_AGGREGATE: &str = "Merge prior runs' results by shared substance";

/// The set of arclite subcommands.
// Parsed once at startup and dispatched immediately — never held in a collection or a hot path — so
// the size gap between `Run` (it carries the synthesis args) and the small deterministic variants is
// irrelevant; boxing it would only fight clap's derive for no real gain.
#[derive(Debug, Subcommand)]
#[allow(clippy::large_enum_variant)]
pub enum Command {
    /// Report runtime, environment, and available tooling.
    Doctor(DoctorArgs),
    /// Check whether a newer arc release is published (this binary vs. the latest release tag).
    Update(UpdateArgs),
    /// Walk a repository and report structured facts about it.
    Inspect(InspectArgs),
    /// Scaffold a repository's `.arc` config (and, with --hook, a pre-push gate).
    Init(InitArgs),
    /// Report the runs currently in flight (the active-run registry).
    Status(StatusArgs),
    /// Open the interactive TUI: a `/` command palette plus a live, self-refreshing run-status view.
    Tui(TuiArgs),
    /// List the rules in play: the active ruleset, its sources, and each rule's provenance.
    Rules(RulesArgs),
    /// List the models each backend's provider API reports available (needs an API key: the
    /// provider's standard env var, or a saved user-layer `api_keys.*` setting).
    Models(ModelsArgs),
    /// Get, set, or list arclite settings (`config list` shows the keys).
    Config(ConfigArgs),
    /// Show the run history, or one run's full result with `<id>` (the completed-run log).
    Log(LogArgs),
    /// Aggregate run stats from the log — runs, blocks, tokens, and cost over the past hour, day,
    /// week, and all time, plus per-command totals.
    Usage(UsageArgs),
    /// Promote a logged run's findings into the repo's `.arc/findings/` ledger — the system writes the
    /// entries (atomically, collision-free), so findings are curated without hand-editing `.arc`.
    Promote(PromoteArgs),
    /// Retire a verify run's `resolved` findings — the system moves them out of the open ledger into
    /// `.arc/findings/resolved/`, the lifecycle's other end (agents invoke arc, never hand-edit `.arc`).
    Retire(RetireArgs),
    /// Generate a shell completion script for `arc` (write it where your shell loads completions).
    Completions(CompletionsArgs),
    // The synthesis verbs are grouped under `run` (`arc run <verb>`): one prompt-differentiated
    // substrate, kept distinct from the deterministic commands above — `run` names the step that
    // spends AI, mirroring arclite's deterministic-until-synthesis spine.
    /// Run an AI synthesis verb: audit, critique, verify, suggest, summarize, extract, evolve, or
    /// aggregate.
    #[command(name = NAME_RUN)]
    Run(RunArgs),
}

/// `arc run <verb>` — the AI synthesis verbs grouped under one subcommand.
#[derive(Debug, clap::Args)]
pub struct RunArgs {
    #[command(subcommand)]
    pub verb: RunVerb,
}

/// The synthesis verbs. Each shares its subcommand name + description with the TUI palette via the
/// `NAME_*`/`VERB_*` consts above, so the launcher (which spawns `arc run <name>`) and `--help` can't
/// drift — `name`/`about` set explicitly rather than left to the variant + a doc-comment.
#[derive(Debug, Subcommand)]
pub enum RunVerb {
    #[command(name = NAME_SUMMARIZE, about = VERB_SUMMARIZE)]
    Summarize(SynthArgs),
    #[command(name = NAME_SUGGEST, about = VERB_SUGGEST)]
    Suggest(SynthArgs),
    #[command(name = NAME_EXTRACT, about = VERB_EXTRACT)]
    Extract(SynthArgs),
    #[command(name = NAME_AUDIT, about = VERB_AUDIT)]
    Audit(SynthArgs),
    #[command(name = NAME_CRITIQUE, about = VERB_CRITIQUE)]
    Critique(SynthArgs),
    #[command(name = NAME_VERIFY, about = VERB_VERIFY)]
    Verify(SynthArgs),
    #[command(name = NAME_EVOLVE, about = VERB_EVOLVE)]
    Evolve(SynthArgs),
    #[command(name = NAME_AGGREGATE, about = VERB_AGGREGATE)]
    Aggregate(SynthArgs),
}

#[derive(Debug, Args)]
pub struct DoctorArgs {}

#[derive(Debug, Args)]
pub struct StatusArgs {}

#[derive(Debug, Args)]
pub struct UpdateArgs {
    /// Download the newer release and install it over the running binary (default: only report).
    #[arg(long)]
    pub apply: bool,
    /// With --apply, reinstall even when already up to date (repair, or re-pull the current build).
    #[arg(long)]
    pub force: bool,
}

/// Arguments for `arc tui`.
#[derive(Debug, Args)]
pub struct TuiArgs {
    /// Seconds between live refreshes of the run registry.
    #[arg(long, value_name = "SECS", default_value_t = crate::commands::tui::DEFAULT_INTERVAL_SECS)]
    pub interval: f64,
}

/// Arguments for `arc models`.
#[derive(Debug, Args)]
pub struct ModelsArgs {
    /// Only this backend's provider (default: every known backend).
    #[arg(long, value_name = "NAME")]
    pub backend: Option<String>,
}

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
        /// The value (validated and typed per key). For the secret keys (`api_keys.*`) omit it:
        /// the value is read from stdin instead, so a credential never rides argv or shell history.
        value: Option<String>,
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
    #[arg(long, value_name = "CMD", conflicts_with = "id")]
    pub command: Option<String>,
    /// Only runs whose repo path contains this (case-insensitive).
    #[arg(long, value_name = "TEXT", conflicts_with = "id")]
    pub repo: Option<String>,
    /// Only runs where the gate blocked.
    #[arg(long, conflicts_with = "id")]
    pub blocked: bool,
    /// List all matching runs, not just the most recent.
    #[arg(long, conflicts_with = "id")]
    pub all: bool,
}

#[derive(Debug, Args)]
pub struct UsageArgs {}

/// Arguments for `arc promote`.
#[derive(Debug, Args)]
pub struct PromoteArgs {
    /// The run id — or a unique id prefix — whose findings to promote (as shown by `arc log`).
    pub run: String,
    /// Preview which findings would be promoted, and where, without writing anything.
    #[arg(long)]
    pub dry_run: bool,
}

/// Arguments for `arc retire`.
#[derive(Debug, Args)]
pub struct RetireArgs {
    /// The verify run id — or a unique id prefix — whose `resolved` verdicts to act on (per `arc log`).
    pub run: String,
    /// Preview which findings would be retired, and where, without moving anything.
    #[arg(long)]
    pub dry_run: bool,
}

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
    /// Drop paths from the included context — gitignore-style patterns (repeatable), matched against
    /// the walked `--include`/`--changed` files. E.g. `--exclude "*.Designer.cs" --exclude ".claude/"`
    /// to skip generated files and session config. Off by default; every pattern is echoed in the run.
    #[arg(long, value_name = "PATTERN")]
    pub exclude: Vec<String>,
    /// Optional compression: cap each included file (and the README/manifests) at N chars.
    /// Default: none — files are read in full; capping is never automatic, and any cut is
    /// surfaced in the run's sources.
    #[arg(long, value_name = "N")]
    pub max_file_chars: Option<usize>,
    /// Add files changed in git (staged, unstaged, or untracked) to the context, alongside any
    /// `--include`. Works with any command. Default: off.
    #[arg(long)]
    pub changed: bool,
    /// Skip the automatic repo scan — the scan summary and the manifests it detects, plus the walk
    /// that builds them — leaving only the README, any `--include`/`--changed` files, and the rules.
    /// For diff-scoped runs (e.g. a pre-push gate) whose cost should track the diff, not a fixed
    /// whole-repo baseline. Default: the scan is included. The skip is echoed in the run's excluded list.
    #[arg(long)]
    pub no_scan: bool,
    /// Feed this repo's open findings ledger (`.arc/findings/open/`) into context, with an instruction
    /// to surface NEW issues beyond the already-recorded ones — so a run hunts past what's known and
    /// re-reports less. Default: off. No ledger → no effect.
    #[arg(long)]
    pub findings: bool,
    /// Feed a logged run's stored structured results into context (repeatable; a unique id prefix
    /// works). Consumed only by the `aggregate` verb, which needs at least two — sameness across
    /// runs is the judgment. Other verbs reject it.
    #[arg(long, value_name = "RUN_ID")]
    pub from: Vec<String>,
    /// Also write the synthesis to `<DIR>/<command>.md` — a self-describing generated doc (the
    /// directory is created if needed). Stdout output is unchanged; `--dry-run` writes nothing.
    #[arg(long, value_name = "DIR")]
    pub output: Option<PathBuf>,
    /// Load the agent's ambient project memory into the synthesis (claude: your user/project
    /// `CLAUDE.md` + auto-memory; codex: the repo's `AGENTS.md`). Default: off — arclite isolates the
    /// run so the reported context is authoritative and reproducible across machines. Enable to
    /// deliberately apply your own ambient standards.
    #[arg(long)]
    pub ambient_memory: bool,
    /// Gate on the command's results: exit non-zero if its structured `results` array is
    /// non-empty — for enforcement in git hooks/CI, where a hook blocks on exit status alone. Opt-in;
    /// requires a verb with structured output, which every verb but `summarize` produces by default
    /// (a schema-validated `results` array + `note` — the canonical output the human view derives
    /// from; `--json` carries it for machines). A prose verb rejects the flag. Off by default.
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
