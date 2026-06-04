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
    Summarize(SummarizeArgs),
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

/// Arguments for `arclite summarize`.
#[derive(Debug, Args)]
pub struct SummarizeArgs {
    /// Path to the repository or directory to assess (defaults to the current directory).
    #[arg(default_value = ".")]
    pub path: PathBuf,
    /// Model to use (a Claude model id). Required for a real call — arclite picks none for you.
    #[arg(long)]
    pub model: Option<String>,
    /// Build and show the prompt + a token/cost estimate WITHOUT calling the model (zero spend).
    #[arg(long)]
    pub dry_run: bool,
    /// Allow a Claude tool during synthesis (repeatable). Default: none — summarize is pure
    /// text synthesis, so it runs with no tools, which is far cheaper. Add only if needed.
    #[arg(long = "allow-tool", value_name = "TOOL")]
    pub allow_tool: Vec<String>,
}
