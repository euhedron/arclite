use serde::Serialize;

use crate::cli::{DoctorArgs, GlobalArgs};
use crate::output::emit;

#[derive(Serialize)]
struct Report {
    arclite: &'static str,
    runtime: Runtime,
    cwd: String,
    tools: Tools,
    logs: Logs,
}

#[derive(Serialize)]
struct Runtime {
    os: &'static str,
    arch: &'static str,
}

#[derive(Serialize)]
struct Tools {
    cargo: Option<String>,
    git: Option<String>,
    claude: Option<String>,
}

/// Where per-run records are logged and how many exist — so logging is discoverable (on by
/// default) without each run announcing it. `path` is `None` only if the home dir is unknown.
#[derive(Serialize)]
struct Logs {
    path: Option<String>,
    runs: Option<usize>,
}

/// Return the trimmed first line of `<cmd> --version`, or `None` if the command
/// is missing or exits non-zero.
fn probe(program: &str) -> Option<String> {
    let output = crate::ai::command(program).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .map(|line| line.trim().to_owned())
}

/// Report runtime, environment, and available tooling. Deterministic — no LLM.
pub fn run(_args: &DoctorArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    let runs = crate::log::count();
    let report = Report {
        arclite: env!("CARGO_PKG_VERSION"),
        runtime: Runtime {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
        },
        cwd: std::env::current_dir()?.display().to_string(),
        tools: Tools {
            cargo: probe("cargo"),
            git: probe("git"),
            claude: probe("claude"),
        },
        logs: Logs {
            path: crate::log::path().map(|p| p.display().to_string()),
            runs: runs.as_ref().ok().copied(),
        },
    };

    let runs_display = match &runs {
        Ok(n) => format!("{n} runs"),
        Err(e) => format!("unreadable: {e}"),
    };
    let human = format!(
        "arclite {}\nos      {} / {}\ncwd     {}\ncargo   {}\ngit     {}\nclaude  {}\nlogs    {} ({})",
        report.arclite,
        report.runtime.os,
        report.runtime.arch,
        report.cwd,
        report.tools.cargo.as_deref().unwrap_or("not found"),
        report.tools.git.as_deref().unwrap_or("not found"),
        report.tools.claude.as_deref().unwrap_or("not found"),
        report.logs.path.as_deref().unwrap_or("unavailable (no home dir)"),
        runs_display,
    );

    emit(&report, &human, global.json)
}
