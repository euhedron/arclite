use anyhow::{Context, bail};
use serde_json::Value;

use crate::cli::{GlobalArgs, LogArgs};
use crate::output::emit;

/// The `log` command.
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
    let text = crate::read_optional(&path)
        .with_context(|| format!("cannot read the run log {}", path.display()))?
        .unwrap_or_default();
    let mut records: Vec<Value> = Vec::new();
    let mut unparsed = 0usize;
    for line in crate::log::record_lines(&text) {
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

/// A string field of a record/run JSON object, or `?` if absent.
fn field(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("?").to_owned()
}

/// The recorded cost (`usage.cost_usd`) of a record/run JSON object, formatted for display —
/// `$?` when absent, so a missing cost can't read as genuine zero spend.
fn cost(v: &Value) -> String {
    v.get("usage")
        .and_then(|u| u.get("cost_usd"))
        .and_then(Value::as_f64)
        .map_or_else(|| "$?".to_owned(), crate::log::cost_display)
}

/// One log record as a compact row — tolerant of older records that predate some fields.
fn row(r: &Value, now: u64) -> String {
    let id = r.get("id").and_then(Value::as_str).unwrap_or("-");
    let ts = r.get("ts").and_then(Value::as_u64).unwrap_or(0);
    let repo_full = field(r, "repo");
    let repo = repo_full
        .rsplit(['/', '\\'])
        .next()
        .expect("rsplit always yields at least one piece");
    let blocked = r.get("blocked").and_then(Value::as_bool).unwrap_or(false);
    format!(
        "{id} · {} · {} · {} · {} · {}{}",
        age(now.saturating_sub(ts)),
        field(r, "command"),
        repo,
        field(r, "model"),
        cost(r),
        if blocked { " · BLOCKED" } else { "" },
    )
}

// Seconds per minute, hour, and day — the bucket thresholds and divisors for the relative age below.
const SECS_PER_MINUTE: u64 = 60;
const SECS_PER_HOUR: u64 = 60 * SECS_PER_MINUTE;
const SECS_PER_DAY: u64 = 24 * SECS_PER_HOUR;

/// A coarse relative age: seconds, minutes, hours, or days.
fn age(secs: u64) -> String {
    match secs {
        s if s < SECS_PER_MINUTE => format!("{s}s ago"),
        s if s < SECS_PER_HOUR => format!("{}m ago", s / SECS_PER_MINUTE),
        s if s < SECS_PER_DAY => format!("{}h ago", s / SECS_PER_HOUR),
        s => format!("{}d ago", s / SECS_PER_DAY),
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
    let meta = format!(
        "{} · {} · {} · {}",
        field(&run, "command"),
        field(&run, "repo"),
        field(&run, "model"),
        cost(&run),
    );
    let body = match v.get("structured") {
        Some(s) if !s.is_null() => serde_json::to_string_pretty(s).unwrap_or_default(),
        _ => v.get("text").and_then(Value::as_str).unwrap_or("").to_owned(),
    };
    format!("{meta}\n\n{body}")
}
