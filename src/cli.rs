use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// Top-level arclite command-line interface.
#[derive(Debug, Parser)]
#[command(
    name = "arc",
    version,
    about = "Agent-first CLI for cross-repo code intelligence and auditing.",
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
    /// Synthesize a brief assessment of a repository via the Claude CLI.
    Summarize(SynthArgs),
    /// Synthesize a prioritized list of suggestions for a repository via the Claude CLI.
    Suggest(SynthArgs),
    /// Extract reusable rules (standards, anti-patterns, principles) from a repository via the Claude CLI.
    Extract(SynthArgs),
    /// Audit a repository against selected rules, reporting only violations, via the Claude CLI.
    Audit(SynthArgs),
    /// Critically review a repo + its docs for quality defects (redundancy, staleness, gaps) via the Claude CLI.
    Critique(SynthArgs),
}

/// Arguments for `arclite doctor`.
#[derive(Debug, Args)]
pub struct DoctorArgs {}

/// Arguments for `arclite inspect`.
#[derive(Debug, Args)]
pub struct InspectArgs {
    /// Path to the repository or directory to inspect (defaults to the current directory).
    #[arg(default_value = ".")]
    pub path: PathBuf,
}

/// Shared arguments for the synthesis commands (`summarize`, `suggest`, `extract`).
#[derive(Debug, Args)]
pub struct SynthArgs {
    /// Path to the repository or directory (defaults to the current directory).
    #[arg(default_value = ".")]
    pub path: PathBuf,
    /// Model to use (a Claude model id). Defaults to the best available (`opus`); set this to
    /// configure *down* for cost. A small model gives unrealistic signal when judging output.
    #[arg(long)]
    pub model: Option<String>,
    /// Build and show the prompt + a token/cost estimate WITHOUT calling the model (zero spend).
    #[arg(long)]
    pub dry_run: bool,
    /// Allow a Claude tool during synthesis (repeatable). Default: none — these commands are pure
    /// text synthesis, so they run with no tools, which is far cheaper. Add only if needed.
    #[arg(long = "allow-tool", value_name = "TOOL")]
    pub allow_tool: Vec<String>,
    /// Directory of Markdown rule files to weigh the synthesis against (anti-patterns, standards,
    /// principles, …). Ad-hoc override that takes precedence over `--ruleset` and the default.
    #[arg(long, value_name = "DIR")]
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
    /// Scope the included context to files changed in git (staged, unstaged, or untracked).
    /// Works with any synthesis command — e.g. a cheap, focused `arc audit --changed`. Default: off.
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
    /// Emit the command's structured output (a schema-validated typed object) instead of prose,
    /// where the command defines one (e.g. `audit` violations, `suggest` a ranked list). Optional;
    /// commands without a structured mode reject it. Compose with `--json` for machine consumption.
    #[arg(long)]
    pub structured: bool,
    /// Gate on the command's findings: exit non-zero (code 2) if its structured findings collection
    /// is non-empty (e.g. `audit` violations, `suggest` suggestions) — for enforcement in git hooks/
    /// CI, where a hook blocks on exit status alone. Opt-in; implies `--structured`; rejected by
    /// commands that emit no findings (e.g. `summarize`). Off by default — no command gates unless asked.
    #[arg(long)]
    pub fail_on_findings: bool,
}
