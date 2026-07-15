use anyhow::{Context, bail};
use serde_json::Value;

use crate::cli::{GlobalArgs, LogArgs};
use crate::log::{SECS_PER_DAY, SECS_PER_HOUR, SECS_PER_MINUTE, field, repo_basename};
use crate::output::emit;

/// The `log` command.
pub fn run(args: &LogArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    if let Some(id) = &args.id {
        show(id, global)
    } else if args.last {
        let (records, unparsed) = matching_records(args)?;
        // With corrupt lines in the log, "newest parsed" may not be "newest run" — disclosed, so
        // --last can't silently answer with an older record (distinguish-absent-from-unreadable).
        if unparsed > 0 {
            eprintln!(
                "arclite: {} — the newest *parsed* run is shown, which may not be the newest run",
                crate::log::unparsed_note(unparsed)
            );
        }
        let newest = records.first().context("no runs match")?;
        let id = newest
            .get("id")
            .and_then(Value::as_str)
            .context("the newest matching run predates the result store (it has no id)")?;
        show(id, global)
    } else {
        list(args, global)
    }
}

/// How many runs `arc log` shows without `--all`.
const DEFAULT_LIMIT: usize = 20;

/// The run records (newest first) that pass the `--command`/`--repo`/`--blocked` filters, plus how
/// many log lines didn't parse.
fn matching_records(args: &LogArgs) -> anyhow::Result<(Vec<Value>, usize)> {
    let (mut records, unparsed) = crate::log::records_newest_first()?;
    records.retain(|r| keep(r, args));
    Ok((records, unparsed))
}

/// Whether one record passes the selection filters.
fn keep(r: &Value, args: &LogArgs) -> bool {
    if let Some(command) = &args.command
        && r.get("command").and_then(Value::as_str) != Some(command)
    {
        return false;
    }
    if let Some(repo) = &args.repo
        && !field(r, "repo")
            .to_lowercase()
            .contains(&repo.to_lowercase())
    {
        return false;
    }
    if args.blocked && !crate::log::is_blocked(r) {
        return false;
    }
    true
}

fn list(args: &LogArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    let (records, unparsed) = matching_records(args)?;
    let total = records.len();
    let shown = if args.all {
        &records[..]
    } else {
        &records[..total.min(DEFAULT_LIMIT)]
    };
    let now = crate::log::now_secs();

    let mut lines: Vec<String> = shown.iter().map(|r| row(r, now)).collect();
    if lines.is_empty() {
        lines.push("no matching runs".to_owned());
    } else if !args.all && total > shown.len() {
        lines.push(format!(
            "… {} older run(s) — `arc log --all` for the rest",
            total - shown.len()
        ));
    }
    // Unparseable lines are surfaced, not silently dropped, so the count can't quietly under-report.
    if unparsed > 0 {
        lines.push(crate::log::unparsed_note(unparsed));
    }
    let payload = serde_json::json!({
        "runs": shown,
        "shown": shown.len(),
        "total": total,
        "unparsed": unparsed,
    });
    emit(&payload, &lines.join("\n"), global.json)
}

/// The recorded cost of a record/run JSON object, formatted for display. When the record carries
/// usage, this is the same rendering the live run report uses (`$X`, or "tokens only (no $)" for a
/// backend like codex that reports tokens but no cost) — so a run reads the same logged as it did
/// live. A record with no usage at all has a genuinely-absent cost, shown `$?` (never a bogus $0).
pub(crate) fn cost(v: &Value) -> String {
    if v.get("usage").is_some() {
        crate::log::cost_or_unavailable(crate::log::record_cost(v))
    } else {
        crate::log::COST_NO_USAGE.to_owned()
    }
}

/// One log record as a compact row — tolerant of older records that predate some fields. Shared with
/// the TUI's `log` view, so the list reads the same in the cockpit as on the CLI.
pub(crate) fn row(r: &Value, now: u64) -> String {
    let id = field(r, "id");
    let age = record_age(r, now);
    let repo_full = field(r, "repo");
    let repo = repo_basename(&repo_full);
    let blocked = crate::log::is_blocked(r);
    // A run that errored (spent but didn't complete) is flagged so failed runs stand out in the list;
    // it's mutually exclusive with a gate verdict (a failed run never reaches the gate).
    let errored = crate::log::is_errored(r);
    // A model the backend never confirmed shows as requested — the list must not present the
    // requested id as the identity that ran (report-the-identity-that-ran).
    let model = if crate::log::model_requested(r) {
        format!(
            "{}{}",
            field(r, "model"),
            crate::log::MODEL_REQUESTED_SUFFIX
        )
    } else {
        field(r, "model")
    };
    format!(
        "{id} · {age} · {} · {} · {model} · {}{}{}",
        field(r, "command"),
        repo,
        cost(r),
        if blocked {
            format!(" · {}", crate::log::gate_label(blocked))
        } else {
            String::new()
        },
        if errored { " · errored" } else { "" },
    )
}

/// A run record's age relative to `now`, from its `ts` field: `"?"` when absent — surfaced, not shown
/// as a bogus age computed from a zero timestamp (matching how the other fields disclose an absent
/// value rather than faking one) — else the coarse relative [`age`]. The single `ts`→age extraction
/// shared by the `log` row and the TUI status tail, so the missing-`ts` handling can't drift.
pub(crate) fn record_age(r: &Value, now: u64) -> String {
    r.get("ts")
        .and_then(Value::as_u64)
        .map_or_else(|| "?".to_owned(), |ts| age(now.saturating_sub(ts)))
}

/// A coarse relative age: seconds, minutes, hours, or days — the private kernel of [`record_age`].
fn age(secs: u64) -> String {
    match secs {
        s if s < SECS_PER_MINUTE => format!("{s}s ago"),
        s if s < SECS_PER_HOUR => format!("{}m ago", s / SECS_PER_MINUTE),
        s if s < SECS_PER_DAY => format!("{}h ago", s / SECS_PER_HOUR),
        s => format!("{}d ago", s / SECS_PER_DAY),
    }
}

/// Days since 1970-01-01 to a civil (year, month, day) — Howard Hinnant's `civil_from_days`
/// algorithm (no timezone/date dependency needed). The literals are its calendar constants:
/// 146097 days per 400-year era, 719468 days from 0000-03-01 to the 1970-01-01 epoch.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (
        yoe + era * 400 + i64::from(month <= 2),
        month as u32,
        day as u32,
    )
}

/// A UNIX timestamp as `YYYY-MM-DD HH:MM UTC` — UTC because it's exact without a timezone
/// dependency, and labeled so it can't be misread as local time. Shared with `promote`, which stamps
/// it into ledger entries as `recorded:`.
pub(crate) fn datetime_utc(secs: u64) -> String {
    let (y, m, d) =
        civil_from_days(i64::try_from(secs / SECS_PER_DAY).expect("fits until year ~25e12"));
    let rem = secs % SECS_PER_DAY;
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        rem / SECS_PER_HOUR,
        (rem % SECS_PER_HOUR) / SECS_PER_MINUTE
    )
}

/// Load a run's stored result by exact id: `Ok(Some(value))` if kept, `Ok(None)` if genuinely absent
/// (a run predating the result store, or made with logging off), `Err` if the store can't be located
/// or the file exists but can't be read/parsed — the absent-vs-unreadable distinction, so a corrupt
/// result isn't read as "not kept". Shared by `show` (CLI) and the TUI's `log` detail view.
pub(crate) fn load_stored(id: &str) -> anyhow::Result<Option<Value>> {
    let path = crate::log::result_path(id)
        .context("cannot determine the result path (no home directory)")?;
    let Some(text) =
        crate::read_optional(&path).with_context(|| format!("cannot read {}", path.display()))?
    else {
        return Ok(None);
    };
    let stored = serde_json::from_str(&text)
        .with_context(|| format!("invalid result file {}", path.display()))?;
    Ok(Some(stored))
}

/// The run record inside a stored result — `Value::Null` if absent (a partial/older store), for the
/// caller to interpret. Single-sourced so promote and the `log` detail view read it the same way.
pub(crate) fn stored_run(stored: &Value) -> Value {
    stored.get("run").cloned().unwrap_or(Value::Null)
}

fn show(id: &str, global: &GlobalArgs) -> anyhow::Result<()> {
    let id = resolve_id(id)?;
    let Some(stored) = load_stored(&id)? else {
        bail!(
            "no stored result for run `{id}` (runs predating the store, or made with logging off, aren't kept)"
        )
    };
    emit(&stored, &stored_human(&stored), global.json)
}

/// Reject a run id that isn't a single safe path segment — it addresses a file in the result store
/// (`<id>.json`), so separators, `..`, or a drive prefix would let a crafted id escape the store.
/// Call this at every boundary an id crosses in from outside — CLI argv via [`resolve_id`], or a log
/// record (a file editable outside the program) via the TUI detail view — so the downstream path
/// joins can treat it as safe.
pub(crate) fn ensure_safe_run_id(id: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !id.is_empty() && !id.contains(['/', '\\', ':']) && !id.contains(".."),
        "invalid run id `{id}`: expected a single path segment (no separators, `..`, or `:`)"
    );
    Ok(())
}

/// Resolve a full run id, or a unique prefix of one, against the result store (exact match wins;
/// an ambiguous prefix errors listing the candidates). An id with no stored entry passes through
/// unchanged so [`show`] reports the authoritative "no stored result" error.
pub(crate) fn resolve_id(prefix: &str) -> anyhow::Result<String> {
    ensure_safe_run_id(prefix)?;
    let Some(dir) = crate::log::results_dir() else {
        return Ok(prefix.to_owned());
    };
    let Some(entries) = crate::read_dir_optional(&dir)
        .with_context(|| format!("cannot read the result store {}", dir.display()))?
    else {
        return Ok(prefix.to_owned());
    };
    let mut matches: Vec<String> = Vec::new();
    for entry in entries {
        let path = entry
            .with_context(|| format!("cannot read an entry in the result store {}", dir.display()))?
            .path();
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if stem == prefix {
            return Ok(stem.to_owned());
        }
        if stem.starts_with(prefix) {
            matches.push(stem.to_owned());
        }
    }
    match matches.as_slice() {
        [] => Ok(prefix.to_owned()),
        [one] => Ok(one.clone()),
        _ => {
            // Cap the candidate listing so a very short prefix doesn't dump the whole store.
            const AMBIGUOUS_LISTED: usize = 8;
            matches.sort();
            let extra = matches.len().saturating_sub(AMBIGUOUS_LISTED);
            matches.truncate(AMBIGUOUS_LISTED);
            bail!(
                "run id prefix `{prefix}` is ambiguous: {}{}",
                matches.join(", "),
                if extra > 0 {
                    format!(", … and {extra} more")
                } else {
                    String::new()
                }
            )
        }
    }
}

/// The string-array field `key` of a run record (`sources`, `tools`, …) as owned strings, or empty.
fn record_strings(run: &Value, key: &str) -> Vec<String> {
    run.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// A stored run for humans: identity, the run's shape and ground-truth usage, the context it was given
/// (the source list + prompt size), then the result body (structured if present, else text). Shared
/// with the TUI's `log` detail view, so a run reads the same there. The verbatim prompt is stored in
/// the result too (its `prompt` field) for full inspection beyond this summary.
pub(crate) fn stored_human(v: &Value) -> String {
    let run = stored_run(v);
    let when = run
        .get("ts")
        .and_then(Value::as_u64)
        .map_or_else(|| "?".to_owned(), datetime_utc);
    let mut meta = format!(
        "{} · {} · arc v{}",
        field(&run, "id"),
        when,
        field(&run, "version")
    );
    // Identity: command, repo (at its commit, when the run recorded one), and the backend/model —
    // with a requested-not-confirmed model id disclosed, same as it showed live.
    let model = if crate::log::model_requested(&run) {
        format!(
            "{}{}",
            field(&run, "model"),
            crate::log::MODEL_REQUESTED_SUFFIX
        )
    } else {
        field(&run, "model")
    };
    meta.push_str(&format!(
        "\n{} · {} · {}/{model}",
        field(&run, "command"),
        crate::display_path(&field(&run, "repo")),
        field(&run, "backend"),
    ));
    if let Some(commit) = run.get("commit").and_then(Value::as_str) {
        meta.push_str(&format!(" · commit {commit}"));
    }
    // A run that errored (spent but didn't complete) reports the failure instead of a gate verdict —
    // it never reached the gate, so showing "gate: passed" would misread.
    if let Some(error) = run.get("error").and_then(Value::as_str) {
        meta.push_str(&format!(" · errored: {error}"));
    } else if run.get("gate").and_then(Value::as_str).is_some() {
        let blocked = crate::log::is_blocked(&run);
        meta.push_str(&format!(" · gate: {}", crate::log::gate_label(blocked)));
    }
    // Run shape: runs (vs. requested), memory isolation, the budget cap, codex reasoning effort.
    let runs = run.get("runs").and_then(Value::as_u64).unwrap_or(1) as usize;
    let requested = run
        .get("runs_requested")
        .and_then(Value::as_u64)
        .map_or(runs, |r| r as usize);
    meta.push('\n');
    if let Some(s) = crate::log::runs_summary(runs, requested) {
        meta.push_str(&format!("{s} · "));
    }
    let budget =
        crate::log::budget_display(run.get("max_budget_usd").and_then(Value::as_f64), requested);
    meta.push_str(&format!(
        "memory={} · budget={budget}",
        field(&run, "memory")
    ));
    if let Some(effort) = run.get("reasoning_effort").and_then(Value::as_str) {
        meta.push_str(&format!(" · reasoning={effort}"));
    }
    // Ground-truth token usage + cost (no fabricated zeros for records predating the usage field).
    let usage = match run.get("usage") {
        Some(u) => {
            let t = crate::log::usage_tokens(u);
            let mut line = format!(
                "tokens: {}",
                crate::log::usage_display(
                    t.input,
                    t.cache_creation,
                    t.cache_read,
                    t.output,
                    crate::log::record_cost(&run),
                    // The record carries the lower-bound marker (absent on single runs and old
                    // records), so a partial fan-out total replays as "≥", same as it showed live.
                    u.get("cost_partial")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                )
            );
            // The same honesty markers the rollup applies: placeholder zeros and mangled fields
            // must not replay as measurements.
            if crate::log::record_spend_unknown(&run) {
                line.push_str(
                    " · spend unknown (the backend returned no usage; zeros are placeholders)",
                );
            }
            if t.malformed > 0 {
                line.push_str(&format!(
                    " · {} usage field(s) absent or non-numeric, read as 0",
                    t.malformed
                ));
            }
            line
        }
        None => format!("cost: {}", crate::log::COST_NO_USAGE),
    };
    meta.push_str(&format!("\n{usage}"));
    // The provided context: the source list and total prompt size (the verbatim prompt is kept in the
    // result's `prompt` field).
    let sources: Vec<String> = record_strings(&run, "sources")
        .iter()
        .map(|s| crate::display_path(s))
        .collect();
    // Disclose absence rather than fabricate a zero: a record predating `prompt_chars` shows "?",
    // not "prompt 0 chars" (which would falsely claim an empty prompt) — matching `ts`/cost above.
    let prompt_chars = run
        .get("prompt_chars")
        .and_then(Value::as_u64)
        .map_or_else(|| "?".to_owned(), |n| n.to_string());
    meta.push_str(&format!(
        "\ncontext ({}): {} · prompt {prompt_chars} chars",
        sources.len(),
        crate::join_or(&sources, "(none)")
    ));
    meta.push_str(&format!(
        "\ntools: {}",
        crate::join_or(&record_strings(&run, "tools"), "none")
    ));
    let body = crate::synth::body_display(
        v.get("structured"),
        v.get("text").and_then(Value::as_str).unwrap_or(""),
    );
    format!("{meta}\n\n{body}")
}
