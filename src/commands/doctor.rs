use serde::Serialize;

use crate::cli::{DoctorArgs, GlobalArgs};
use crate::output::emit;

#[derive(Serialize)]
struct Report {
    arclite: &'static str,
    runtime: Runtime,
    cwd: String,
    tools: Tools,
}

#[derive(Serialize)]
struct Runtime {
    os: &'static str,
    arch: &'static str,
}

#[derive(Serialize)]
struct Tools {
    git: Option<String>,
    claude: Option<String>,
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
    let report = Report {
        arclite: env!("CARGO_PKG_VERSION"),
        runtime: Runtime {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
        },
        cwd: std::env::current_dir()?.display().to_string(),
        tools: Tools {
            git: probe("git"),
            claude: probe("claude"),
        },
    };

    let human = format!(
        "arclite {}\nos      {} / {}\ncwd     {}\ngit     {}\nclaude  {}",
        report.arclite,
        report.runtime.os,
        report.runtime.arch,
        report.cwd,
        report.tools.git.as_deref().unwrap_or("not found"),
        report.tools.claude.as_deref().unwrap_or("not found"),
    );

    emit(&report, &human, global.json)
}
