use anyhow::{Context, bail};
use serde_json::Value;

use crate::cli::{GlobalArgs, LogArgs};
use crate::output::emit;

/// Show the run history, or one run's full result. Lists from the run log
/// (`~/.arc/logs/runs.jsonl`) and fetches a single run from the result store
/// (`~/.arc/logs/results/<id>.json`).
pub fn run(args: &LogArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    match &args.id {
        Some(id) => show(id, global),
        None => list(args.all, global),
    }
}

/// How many runs `arc log` shows without `--all`.
const DEFAULT_LIMIT: usize = 20;

fn list(all: bool, global: &GlobalArgs) -> anyhow::Result<()> {
    let path = crate::log::path().context("cannot determine the run-log path")?;
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(e).with_context(|| format!("cannot read the run log {}", path.display()));
        }
    };
    let mut records: Vec<Value> = Vec::new();
    let mut unparsed = 0usize;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        match serde_json::from_str::<Value>(line) {
            Ok(v) => records.push(v),
            Err(_) => unparsed += 1,
        }
    }
    records.reverse(); // newest first (the log is append-only)
    let total = records.len();
    let shown = if all {
        &records[..]
    } else {
        &records[..total.min(DEFAULT_LIMIT)]
    };
    let now = crate::log::now_secs();

    let mut lines: Vec<String> = shown.iter().map(|r| row(r, now)).collect();
    if lines.is_empty() {
        lines.push("no runs logged yet".to_owned());
    } else if !all && total > shown.len() {
        lines.push(format!(
            "… {} older run(s) — `arc log --all` for the rest",
            total - shown.len()
        ));
    }
    // Unparseable lines are surfaced, not silently dropped, so the count can't quietly under-report.
    if unparsed > 0 {
        lines.push(format!("{unparsed} unparseable log line(s) skipped"));
    }
    let payload = serde_json::json!({
        "runs": shown,
        "shown": shown.len(),
        "total": total,
        "unparsed": unparsed,
    });
    emit(&payload, &lines.join("\n"), global.json)
}

/// One log record as a compact row — tolerant of older records that predate some fields.
fn row(r: &Value, now: u64) -> String {
    let field = |key: &str| r.get(key).and_then(Value::as_str).unwrap_or("?").to_owned();
    let id = r.get("id").and_then(Value::as_str).unwrap_or("-");
    let ts = r.get("ts").and_then(Value::as_u64).unwrap_or(0);
    let repo_full = field("repo");
    let repo = repo_full.rsplit(['/', '\\']).next().unwrap_or(&repo_full);
    let cost = r
        .get("usage")
        .and_then(|u| u.get("cost_usd"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let blocked = r.get("blocked").and_then(Value::as_bool).unwrap_or(false);
    format!(
        "{id} · {} · {} · {} · {} · ${cost:.4}{}",
        age(now.saturating_sub(ts)),
        field("command"),
        repo,
        field("model"),
        if blocked { " · BLOCKED" } else { "" },
    )
}

/// A coarse relative age: seconds, minutes, hours, or days.
fn age(secs: u64) -> String {
    match secs {
        s if s < 60 => format!("{s}s ago"),
        s if s < 3600 => format!("{}m ago", s / 60),
        s if s < 86400 => format!("{}h ago", s / 3600),
        s => format!("{}d ago", s / 86400),
    }
}

fn show(id: &str, global: &GlobalArgs) -> anyhow::Result<()> {
    let path = crate::log::result_path(id).context("cannot determine the result path")?;
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            bail!(
                "no stored result for run `{id}` (runs predating the store, or made with logging off, aren't kept)"
            )
        }
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
    };
    let stored: Value = serde_json::from_str(&text)
        .with_context(|| format!("invalid result file {}", path.display()))?;
    emit(&stored, &stored_human(&stored), global.json)
}

/// A stored run for humans: a metadata line, then the result body (structured if present, else text).
fn stored_human(v: &Value) -> String {
    let run = v.get("run").cloned().unwrap_or(Value::Null);
    let field = |key: &str| run.get(key).and_then(Value::as_str).unwrap_or("?").to_owned();
    let cost = run
        .get("usage")
        .and_then(|u| u.get("cost_usd"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let meta = format!(
        "{} · {} · {} · ${cost:.4}",
        field("command"),
        field("repo"),
        field("model"),
    );
    let body = match v.get("structured") {
        Some(s) if !s.is_null() => serde_json::to_string_pretty(s).unwrap_or_default(),
        _ => v.get("text").and_then(Value::as_str).unwrap_or("").to_owned(),
    };
    format!("{meta}\n\n{body}")
}
