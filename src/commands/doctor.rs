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
    cargo: ToolStatus,
    git: ToolStatus,
    /// Each known synthesis backend ([`crate::ai::KNOWN_BACKENDS`]) and its detected status — probed
    /// from that one set, so a new backend is checked here automatically rather than silently missed.
    backends: Vec<BackendTool>,
}

#[derive(Serialize)]
struct BackendTool {
    name: String,
    status: ToolStatus,
}

/// The detected state of an external tool. Three outcomes kept distinct, per the absent-vs-failed
/// distinction: it ran and reported a version; it's genuinely not installed; or it exists but could
/// not be run — an unreadable shim, a spawn error, or a non-zero/empty `--version` — the last never
/// collapsed into "absent", so a broken tool is never reported as merely missing.
#[derive(Serialize)]
#[serde(tag = "state", content = "detail", rename_all = "snake_case")]
enum ToolStatus {
    Version(String),
    Absent,
    Failed(String),
}

impl ToolStatus {
    /// One-line rendering, parameterized only by what "absent" should say (backends qualify it with
    /// which `--backend` would need the tool); the version and failed cases are shared, so the absent
    /// and failed states stay visibly distinct everywhere `doctor` prints a tool.
    fn display(&self, absent: &str) -> String {
        match self {
            ToolStatus::Version(version) => version.clone(),
            ToolStatus::Absent => absent.to_owned(),
            ToolStatus::Failed(error) => format!("error: {error}"),
        }
    }
}

/// Where per-run records are logged and how many exist — so logging is discoverable (on by
/// default) without each run announcing it. `path` is `None` only if the home dir is unknown.
#[derive(Serialize)]
struct Logs {
    path: Option<String>,
    runs: Option<usize>,
    /// Why `runs` is absent when the log exists but can't be read — carried in the JSON too, not
    /// only the human view, so the machine-readable output doesn't drop the failure.
    error: Option<String>,
}

/// Probe an external tool's `--version`, distinguishing the three outcomes the absent-vs-failed
/// distinction demands: [`ToolStatus::Version`] when it runs and reports one, [`ToolStatus::Absent`]
/// when it genuinely isn't installed (a `NotFound` spawn), and [`ToolStatus::Failed`] when it exists
/// but can't be prepared, spawned, or exits non-zero — never collapsing a broken tool into "absent".
fn probe(program: &str) -> ToolStatus {
    let mut command = match crate::ai::command(program) {
        Ok(command) => command,
        Err(error) => return ToolStatus::Failed(format!("{error:#}")),
    };
    let output = match command.arg("--version").output() {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return ToolStatus::Absent,
        Err(error) => return ToolStatus::Failed(error.to_string()),
    };
    if !output.status.success() {
        return ToolStatus::Failed(format!("exited with {}", output.status));
    }
    match String::from_utf8_lossy(&output.stdout).lines().next() {
        Some(line) => ToolStatus::Version(line.trim().to_owned()),
        None => ToolStatus::Failed("ran but printed no version".to_owned()),
    }
}

/// The `doctor` command.
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
            backends: crate::ai::KNOWN_BACKENDS
                .iter()
                .map(|&name| BackendTool {
                    name: name.to_owned(),
                    status: probe(name),
                })
                .collect(),
        },
        logs: Logs {
            path: crate::log::path().map(|p| p.display().to_string()),
            runs: runs.as_ref().ok().copied(),
            error: runs.as_ref().err().map(std::string::ToString::to_string),
        },
    };

    let runs_display = match &runs {
        Ok(n) => format!("{n} runs"),
        Err(e) => format!("unreadable: {e}"),
    };
    let mut human = format!(
        "arclite {}\nos      {} / {}\ncwd     {}\ncargo   {}\ngit     {}",
        report.arclite,
        report.runtime.os,
        report.runtime.arch,
        report.cwd,
        report.tools.cargo.display("not found"),
        report.tools.git.display("not found"),
    );
    for b in &report.tools.backends {
        // A non-default backend is optional, so qualify its "not found"; a present-but-broken one
        // still surfaces as an error (via `display`), never as merely missing.
        let absent = if b.name == crate::ai::DEFAULT_BACKEND {
            "not found".to_owned()
        } else {
            format!("not found (needed only for --backend {})", b.name)
        };
        human.push_str(&format!("\n{:<8}{}", b.name, b.status.display(&absent)));
    }
    human.push_str(&format!(
        "\nlogs    {} ({runs_display})",
        report
            .logs
            .path
            .as_deref()
            .unwrap_or("unavailable (no home dir)"),
    ));

    emit(&report, &human, global.json)
}
