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
    gate: Gate,
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

/// The repo root containing the cwd: `Ok(Some(root))` inside a work tree, `Ok(None)` when git runs and
/// reports we are not in one (its benign verdict), `Err` when git cannot be run at all or runs but fails
/// for any other reason — so a broken or absent git is surfaced, not collapsed into "not a repo".
/// (`git config --get` outcomes, which need exit 1 separated from >1, use [`crate::git_config_get`].)
fn git_repo_root() -> anyhow::Result<Option<String>> {
    let output = crate::ai::command("git")?
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|e| anyhow::anyhow!("could not run git: {e}"))?;
    if output.status.success() {
        return Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        ));
    }
    // `rev-parse` exits 128 both for "not a git repository" (git's benign not-in-a-work-tree verdict)
    // and for a genuine fault, so the message is the only discriminator: the standard not-a-repo line
    // reads as absent; anything else is a broken git we surface rather than mask as "not a repo".
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("not a git repository") {
        Ok(None)
    } else {
        Err(anyhow::anyhow!(
            "git rev-parse --show-toplevel failed ({}): {}",
            output.status,
            stderr.trim()
        ))
    }
}

/// A pre-push hook's state: present and invoking `arc` (the arc gate is wired), present without it,
/// present but unreadable (kept distinct from absent), or no hook at all.
#[derive(Serialize)]
#[serde(tag = "state", content = "detail", rename_all = "snake_case")]
enum HookStatus {
    InvokesArc,
    NoArc,
    Unreadable(String),
    Absent,
}

/// The pre-push gate's wiring in the cwd repo: where git looks for hooks, and that hook's state.
#[derive(Serialize)]
struct Gate {
    /// `false` when the cwd isn't inside a git repo — then there is no gate to report.
    in_repo: bool,
    /// The `core.hooksPath` value if set, else `None` for git's `.git/hooks` default.
    hooks_path: Option<String>,
    /// The pre-push hook's state; `None` only when not in a repo (or the probe errored).
    pre_push: Option<HookStatus>,
    /// A real failure probing the gate (e.g. a corrupt git config) — surfaced, not collapsed into
    /// "unset" / "no hook".
    error: Option<String>,
}

/// Inspect the cwd repo's pre-push gate so `doctor` shows whether the arc gate is wired in — the
/// status that otherwise takes hand-probing `core.hooksPath` and reading the hook file.
fn gate_status() -> Gate {
    let root = match git_repo_root() {
        Ok(Some(root)) => root,
        // git ran and reported we're not in a work tree — the benign verdict.
        Ok(None) => {
            return Gate {
                in_repo: false,
                hooks_path: None,
                pre_push: None,
                error: None,
            };
        }
        // git itself couldn't run — a real failure, surfaced rather than read as "not a repo".
        Err(e) => {
            return Gate {
                in_repo: false,
                hooks_path: None,
                pre_push: None,
                error: Some(format!("{e:#}")),
            };
        }
    };
    let root = std::path::Path::new(&root);
    // A corrupt/locked config (git exit >1) is surfaced as an error, not masked as "unset".
    let hooks_path = match crate::git_config_get(root, "core.hooksPath") {
        Ok(hooks_path) => hooks_path,
        Err(e) => {
            return Gate {
                in_repo: true,
                hooks_path: None,
                pre_push: None,
                error: Some(format!("{e:#}")),
            };
        }
    };
    let hooks_dir = match &hooks_path {
        Some(p) => crate::resolve_path(root, std::path::Path::new(p)),
        None => root.join(".git").join("hooks"),
    };
    // The binary name to detect is single-sourced in `cli::binary_name` (derived from clap's
    // `#[command(name)]`), so a rename can't stale this detection.
    let bin = crate::cli::binary_name();
    let pre_push = match crate::read_optional(&hooks_dir.join("pre-push")) {
        Ok(Some(body)) if body.contains(&format!("{bin} ")) => HookStatus::InvokesArc,
        Ok(Some(_)) => HookStatus::NoArc,
        Ok(None) => HookStatus::Absent,
        Err(e) => HookStatus::Unreadable(e.to_string()),
    };
    Gate {
        in_repo: true,
        hooks_path,
        pre_push: Some(pre_push),
        error: None,
    }
}

/// Width of `doctor`'s human-output label column, so the value column aligns — named (like tui's
/// column-width constants) rather than implied by hand-spaced labels plus a matching bare literal.
const LABEL_WIDTH: usize = 8;

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
        gate: gate_status(),
    };

    let runs_display = match &runs {
        Ok(n) => format!("{n} runs"),
        Err(e) => format!("unreadable: {e}"),
    };
    // One label column at a single-sourced width, so values align without hand-spaced labels that
    // must be kept in sync with a bare alignment literal by eye.
    let row = |label: &str, value: &str| crate::labeled_row(label, value, LABEL_WIDTH);
    let mut lines = vec![
        row("arclite", report.arclite),
        row(
            "os",
            &format!("{} / {}", report.runtime.os, report.runtime.arch),
        ),
        row("cwd", &crate::display_path(&report.cwd)),
        row("cargo", &report.tools.cargo.display("not found")),
        row("git", &report.tools.git.display("not found")),
    ];
    for b in &report.tools.backends {
        // A non-default backend is optional, so qualify its "not found"; a present-but-broken one
        // still surfaces as an error (via `display`), never as merely missing.
        let absent = if b.name == crate::ai::DEFAULT_BACKEND {
            "not found".to_owned()
        } else {
            format!("not found (needed only for --backend {})", b.name)
        };
        lines.push(row(&b.name, &b.status.display(&absent)));
    }
    lines.push(row(
        "logs",
        &format!(
            "{} ({runs_display})",
            crate::display_path(
                report
                    .logs
                    .path
                    .as_deref()
                    .unwrap_or("unavailable (no home dir)"),
            )
        ),
    ));
    let gate_line = if let Some(e) = &report.gate.error {
        format!("error: {e}")
    } else if !report.gate.in_repo {
        "not a git repository".to_owned()
    } else {
        let where_ = report.gate.hooks_path.as_deref().map_or_else(
            || "default .git/hooks".to_owned(),
            |p| format!("core.hooksPath={p}"),
        );
        let state = match &report.gate.pre_push {
            Some(HookStatus::InvokesArc) => "pre-push invokes arc".to_owned(),
            Some(HookStatus::NoArc) => "pre-push present (no arc)".to_owned(),
            Some(HookStatus::Unreadable(e)) => format!("pre-push unreadable: {e}"),
            Some(HookStatus::Absent) | None => "no pre-push hook".to_owned(),
        };
        format!("{state} · {where_}")
    };
    lines.push(row("gate", &gate_line));
    let human = lines.join("\n");

    emit(&report, &human, global.json)
}
