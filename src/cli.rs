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
}

/// Arguments for `arclite doctor`.
#[derive(Debug, Args)]
pub struct DoctorArgs {}
