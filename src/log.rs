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

/// The four token counts of a record's `usage` object — the single place that knows those JSON
/// field names, so the `arc usage` rollup (which sums them) and the stored-run detail (which
/// renders them) can't drift on the key set. The writer serializes all four fields unconditionally,
/// so in a well-formed record every key is present and numeric; an absent key and a non-numeric one
/// are equally suspect — both read 0 and both count into `malformed`, so damage is disclosed rather
/// than indistinguishable from genuine zero consumption.
pub struct TokenCounts {
    pub input: u64,
    pub cache_creation: u64,
    pub cache_read: u64,
    pub output: u64,
    pub malformed: usize,
}

/// Read the four token counts out of a `usage` JSON object (as nested in a run record).
pub fn usage_tokens(usage: &serde_json::Value) -> TokenCounts {
    let mut malformed = 0usize;
    let mut n = |key: &str| match usage.get(key).and_then(serde_json::Value::as_u64) {
        Some(v) => v,
        None => {
            malformed += 1;
            0
        }
    };
    let input = n("input_tokens");
    let cache_creation = n("cache_creation_input_tokens");
    let cache_read = n("cache_read_input_tokens");
    let output = n("output_tokens");
    TokenCounts {
        input,
        cache_creation,
        cache_read,
        output,
        malformed,
    }
}

/// Whether a run record's spend is *unknown* (the backend returned no usage; the recorded zeros are
/// placeholders) — read from the recorded `usage.spend_unknown`, absent on records that predate the
/// field. The rollup counts these separately from measured token sums.
pub fn record_spend_unknown(record: &serde_json::Value) -> bool {
    record
        .pointer("/usage/spend_unknown")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// A token+cost tally formatted for display — the single statement of the
/// `in/cache-write/cache-read/out | $` line the run report and `arc usage` share. `cost_partial`
/// marks a fan-out sum where some members' dollar cost was unknown: the figure is a lower bound and
/// is shown as one (`≥ $X …`), never presented as the exact total.
pub fn usage_display(
    input: u64,
    cache_write: u64,
    cache_read: u64,
    output: u64,
    cost_usd: Option<f64>,
    cost_partial: bool,
) -> String {
    let cost = if cost_partial {
        format!(
            "≥ {} (a member's cost was unknown)",
            cost_or_unavailable(cost_usd)
        )
    } else {
        cost_or_unavailable(cost_usd)
    };
    format!("in {input}  cache-write {cache_write}  cache-read {cache_read}  out {output} | {cost}")
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

/// Whether a run record carries the two non-clean outcomes — gate-blocked and errored — read from the
/// keys the run report and `arc usage` both key off, so the reading lives in one place rather than
/// open-coded per consumer. A record can be both (a gate that blocked after an error), so these stay
/// independent predicates, not one enum.
pub fn is_blocked(record: &serde_json::Value) -> bool {
    record.get("blocked").and_then(serde_json::Value::as_bool) == Some(true)
}

/// See [`is_blocked`]: whether the run carried an error payload.
pub fn is_errored(record: &serde_json::Value) -> bool {
    record.get("error").is_some()
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

/// Whether a run record's model id was merely *requested* (the backend echoed no model id) rather
/// than response-confirmed — read from the recorded `usage.model_source`, absent on records that
/// predate the field (treated as confirmed, matching their era's semantics). One reader shared by
/// every logged view, so no surface presents an unconfirmed id as the identity that ran.
pub fn model_requested(record: &serde_json::Value) -> bool {
    record
        .pointer("/usage/model_source")
        .and_then(serde_json::Value::as_str)
        == Some("requested")
}

/// The compact suffix logged views hang on a requested-not-confirmed model id — one wording, shared.
pub const MODEL_REQUESTED_SUFFIX: &str = " (requested)";

/// The recorded string form of a repo path — the one conversion the run record and the active-run
/// marker share, and the form ledger commands (`promote`/`retire`) later reopen as a path. Not
/// display formatting: the round-trip is byte-exact, guaranteed by the boundary — every repo path
/// enters through `resolve_root`, which rejects non-UTF-8 before any command runs, precisely so
/// stored state can never address a different path than the one judged.
pub fn repo_record_string(dir: &std::path::Path) -> String {
    dir.to_str()
        .expect("repo paths are validated UTF-8 at the CLI boundary (commands::resolve_root)")
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

/// The current instant as a duration since the UNIX epoch — the one `SystemTime::now()` read the time
/// helpers below share, so the "clock before the epoch" invariant lives in a single place.
fn since_epoch() -> std::time::Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before the UNIX epoch")
}

/// Current UNIX time in seconds.
pub fn now_secs() -> u64 {
    since_epoch().as_secs()
}

/// The current instant's sub-second nanoseconds — opaque entropy appended to a run id so two runs that
/// share a second and a pid (a reused pid across the concurrent sessions on a shared `~/.arc`) don't
/// collide on the result-store key and overwrite each other.
pub fn now_subsec_nanos() -> u32 {
    since_epoch().subsec_nanos()
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
        // ONE `write` call of the record *and* its newline: the kernel atomically positions each
        // single write at EOF under O_APPEND, so concurrent sessions logging to the shared
        // `runs.jsonl` can't interleave *within* a call — a guarantee `write_all` would forfeit,
        // since its retry loop may split the buffer across calls. A rare partial write is therefore
        // surfaced as this best-effort append's failure (the log reader already tolerates and
        // discloses an unparsable line) instead of silently continued into an interleaving risk.
        let buf = format!("{line}\n");
        let n = file.write(buf.as_bytes())?;
        if n != buf.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                format!(
                    "partial append ({n} of {} bytes) — the record line may be truncated",
                    buf.len()
                ),
            ));
        }
        Ok(())
    })
}

/// Store one run's full result at [`result_path`] (best-effort, like [`append`]). Returns the path
/// written, or `None` if it couldn't be stored. The id names the file in a shared store, so it's
/// claimed with an exclusive create: an id collision surfaces through the best-effort warning
/// (`AlreadyExists`) instead of silently overwriting another run's stored result.
pub fn store_result<T: Serialize>(id: &str, content: &T) -> Option<PathBuf> {
    let path = result_path(id)?;
    let body = serde_json::to_string_pretty(content).expect("a run result serializes");
    write_best_effort(path, "run result not stored", |p| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(p)?;
        file.write_all(body.as_bytes())
    })
}
