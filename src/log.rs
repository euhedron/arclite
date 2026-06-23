//! Append-only per-run logging to `~/.arc/logs/runs.jsonl` — a record of every AI run (params,
//! context, tokens, cost), plus each run's full result at `~/.arc/logs/results/<id>.json`. On by
//! default; disable via `defaults.logging = false`. A write failure warns but never fails the command.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use serde::Serialize;

/// Seconds per minute, hour, and day — the time constants shared by `arc log`'s relative ages and
/// `arc usage`'s window sums.
pub const SECS_PER_MINUTE: u64 = 60;
pub const SECS_PER_HOUR: u64 = 60 * SECS_PER_MINUTE;
pub const SECS_PER_DAY: u64 = 24 * SECS_PER_HOUR;

/// The disclosure line for log lines [`records`] couldn't parse — one wording for every consumer.
pub fn unparsed_note(unparsed: usize) -> String {
    format!("{unparsed} unparseable log line(s) skipped")
}

/// The gate-outcome label for a `blocked` flag — one wording shared by the run report and the
/// stored-run view (and the `arc log` row, which shows it only when blocked).
pub fn gate_label(blocked: bool) -> &'static str {
    if blocked { "BLOCKED" } else { "passed" }
}

/// A cost formatted for display — the single statement of the dollar four-decimal format.
pub fn cost_display(cost_usd: f64) -> String {
    format!("${cost_usd:.4}")
}

/// A cost that may be unavailable: `Some` renders as dollars, `None` as the case where the backend
/// reports token usage but no dollar cost (codex). One wording, shared by the run report and `arc usage`.
pub fn cost_or_unavailable(cost_usd: Option<f64>) -> String {
    cost_usd.map_or_else(|| "tokens only (no $)".to_owned(), cost_display)
}

/// The cost shown for a record with no `usage` object at all — a genuinely-absent cost, distinct from
/// a present-but-costless run's "tokens only (no $)". Single-sourced (like the renderers around it) so
/// the sentinel can't drift between `arc log`'s row and its detail view.
pub const COST_NO_USAGE: &str = "$?";

/// The four token counts of a record's `usage` object (0 for any absent field) — the single place
/// that knows those JSON field names, so the `arc usage` rollup (which sums them) and the stored-run
/// detail (which renders them) can't drift on the key set.
pub struct TokenCounts {
    pub input: u64,
    pub cache_creation: u64,
    pub cache_read: u64,
    pub output: u64,
}

/// Read the four token counts out of a `usage` JSON object (as nested in a run record).
pub fn usage_tokens(usage: &serde_json::Value) -> TokenCounts {
    let n = |key: &str| {
        usage
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    };
    TokenCounts {
        input: n("input_tokens"),
        cache_creation: n("cache_creation_input_tokens"),
        cache_read: n("cache_read_input_tokens"),
        output: n("output_tokens"),
    }
}

/// A token+cost tally formatted for display — the single statement of the
/// `in/cache-write/cache-read/out | $` line the run report and `arc usage` share.
pub fn usage_display(
    input: u64,
    cache_write: u64,
    cache_read: u64,
    output: u64,
    cost_usd: Option<f64>,
) -> String {
    format!(
        "in {input}  cache-write {cache_write}  cache-read {cache_read}  out {output} | {}",
        cost_or_unavailable(cost_usd)
    )
}

/// An optional cost cap for display — `none` when uncapped. The cap is *per run*, so across a
/// `--runs N` fan-out (N > 1) the aggregate worst-case exposure is shown too: a user who set a
/// budget shouldn't be surprised that N concurrent runs can each spend up to it. Shared by the live
/// run report and the stored-run view.
pub fn budget_display(cap: Option<f64>, runs: usize) -> String {
    match cap {
        None => "none".to_owned(),
        Some(c) if runs > 1 => format!(
            "{}/run (≤ {} across {runs} runs)",
            cost_display(c),
            cost_display(c * runs as f64)
        ),
        Some(c) => cost_display(c),
    }
}

/// A multi-run fan-out's summary for the run report and the stored-run replay — `runs=N`, or
/// `runs=N (M ok, K failed)` when some were dropped — or `None` for a single run. Single-sourced so
/// the two views can't drift in how they show the fan-out, its drops, or (sized by the same attempted
/// count) the worst-case budget exposure.
pub fn runs_summary(succeeded: usize, requested: usize) -> Option<String> {
    if requested <= 1 {
        None
    } else if succeeded == requested {
        Some(format!("runs={requested}"))
    } else {
        Some(format!(
            "runs={requested} ({succeeded} ok, {} failed)",
            requested - succeeded
        ))
    }
}

/// The recorded cost of a run record (`usage.cost_usd`), `None` when absent — the single accessor,
/// so every reader handles absence deliberately (display `$?`, exclude-and-count in sums) rather
/// than silently zeroing what would read as genuine zero spend.
pub fn record_cost(record: &serde_json::Value) -> Option<f64> {
    record
        .pointer("/usage/cost_usd")
        .and_then(serde_json::Value::as_f64)
}

/// A string field of a run record, or the `?` sentinel if absent — the shared accessor for the
/// record shape that `arc log` and `arc usage` both read, so the sentinel can't drift between them.
pub fn field(record: &serde_json::Value, key: &str) -> String {
    record
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("?")
        .to_owned()
}

/// The last path segment of a repo path — the compact way `arc log` and the TUI show *which* repo a
/// run targeted (the full path stays in `arc status`). `rsplit` always yields at least one piece, so
/// this is total.
pub fn repo_basename(repo: &str) -> &str {
    repo.rsplit(['/', '\\'])
        .next()
        .expect("rsplit always yields at least one piece")
}

/// Current UNIX time in seconds.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before the UNIX epoch")
        .as_secs()
}

/// The arclite logs directory, `~/.arc/logs` — the single source the run log and the result store
/// both build on (`None` only if the home directory can't be determined).
fn logs_dir() -> Option<PathBuf> {
    Some(crate::arc_home()?.join("logs"))
}

/// Path of the run log, `~/.arc/logs/runs.jsonl`. Both [`append`] and `doctor` use it.
pub fn path() -> Option<PathBuf> {
    Some(logs_dir()?.join("runs.jsonl"))
}

/// The result-store directory, `~/.arc/logs/results` — one `<id>.json` per run.
pub fn results_dir() -> Option<PathBuf> {
    Some(logs_dir()?.join("results"))
}

/// The file storing one run's full result, `~/.arc/logs/results/<id>.json` — read by `arc log <id>`.
pub fn result_path(id: &str) -> Option<PathBuf> {
    Some(results_dir()?.join(format!("{id}.json")))
}

/// The record lines of a run-log `text`: non-blank lines, one JSON record each. The single
/// definition of "a record line" — both [`count`] and [`records`] build on it, so the
/// record-per-line format lives in one place rather than drifting between them.
pub fn record_lines(text: &str) -> impl Iterator<Item = &str> + '_ {
    text.lines().filter(|l| !l.trim().is_empty())
}

/// All run records, in log (oldest-first) order, plus how many lines didn't parse — surfaced, never
/// dropped. The one loader `arc log` and `arc usage` share; an absent log is just zero records.
pub fn records() -> anyhow::Result<(Vec<serde_json::Value>, usize)> {
    let path = path().context("cannot determine the run-log path")?;
    let text = crate::read_optional(&path)
        .with_context(|| format!("cannot read the run log {}", path.display()))?
        .unwrap_or_default();
    let mut records = Vec::new();
    let mut unparsed = 0usize;
    for line in record_lines(&text) {
        match serde_json::from_str(line) {
            Ok(v) => records.push(v),
            Err(_) => unparsed += 1,
        }
    }
    Ok((records, unparsed))
}

/// Run records newest-first (the log is append-only, so the latest is last) with the unparsed-line
/// count — the order both `arc log` and the TUI's `log` view present, single-sourced so the two can't
/// drift on it.
pub fn records_newest_first() -> anyhow::Result<(Vec<serde_json::Value>, usize)> {
    let (mut records, unparsed) = records()?;
    records.reverse();
    Ok((records, unparsed))
}

/// Number of run records currently logged — for `doctor`. `Ok(0)` when the log is absent (no runs
/// yet), `Ok(n)` for a readable log, and `Err` when it exists but can't be read: an unreadable log
/// is surfaced distinctly rather than silently shown as 0, which would hide a dropped/corrupt log.
pub fn count() -> std::io::Result<usize> {
    let Some(p) = path() else { return Ok(0) };
    Ok(crate::read_optional(&p)?.map_or(0, |text| record_lines(&text).count()))
}

/// Create `path`'s parent directory and run `write`, returning `Some(path)` on success. A failure
/// warns (prefixed with `what`) and returns `None` — observability writes never fail the command.
fn write_best_effort(
    path: PathBuf,
    what: &str,
    write: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Option<PathBuf> {
    let result = (|| -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        write(&path)
    })();
    match result {
        Ok(()) => Some(path),
        Err(e) => {
            eprintln!("arclite: {what} ({}): {e}", path.display());
            None
        }
    }
}

/// Append `record` as one JSON line to the [`path`] run log, returning the path written, or `None`
/// (with a warning) if it can't be — logging never breaks the command.
pub fn append<T: Serialize>(record: &T) -> Option<PathBuf> {
    let Some(target) = path() else {
        eprintln!("arclite: run not logged (cannot determine the home directory)");
        return None;
    };
    let line = serde_json::to_string(record).expect("a run record serializes");
    write_best_effort(target, "run not logged", |p| {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)?;
        writeln!(file, "{line}")
    })
}

/// Store one run's full result at [`result_path`] (best-effort, like [`append`]). Returns the path
/// written, or `None` if it couldn't be stored.
pub fn store_result<T: Serialize>(id: &str, content: &T) -> Option<PathBuf> {
    let path = result_path(id)?;
    let body = serde_json::to_string_pretty(content).expect("a run result serializes");
    write_best_effort(path, "run result not stored", |p| std::fs::write(p, &body))
}
