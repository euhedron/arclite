use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// Top-level arclite command-line interface.
#[derive(Debug, Parser)]
#[command(
    name = "arclite",
    version,
    about = "Agent-first CLI for cross-repo code intelligence and auditing."
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

/// Shared arguments for the synthesis commands (`summarize`, `suggest`).
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
    /// Directory of Markdown rule files to weigh the synthesis against (anti-patterns,
    /// standards, principles, …). Each rule's text is included in the AI's context.
    #[arg(long, value_name = "DIR")]
    pub rules: Option<PathBuf>,
    /// Include a file or directory in the context (repeatable; capped + bounded). Directories
    /// are walked gitignore-aware — e.g. `--include src`. Default: none.
    #[arg(long, value_name = "PATH")]
    pub include: Vec<PathBuf>,
}
