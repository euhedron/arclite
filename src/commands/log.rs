use anyhow::{Context, bail};
use serde_json::Value;

use crate::cli::{GlobalArgs, LogArgs};
use crate::log::{field, SECS_PER_DAY, SECS_PER_HOUR, SECS_PER_MINUTE};
use crate::output::emit;

/// The `log` command.
pub fn run(args: &LogArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    if let Some(id) = &args.id {
        show(id, global)
    } else if args.last {
        let (records, _) = matching_records(args)?;
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
    let (mut records, unparsed) = crate::log::records()?;
    records.retain(|r| keep(r, args));
    records.reverse(); // newest first (the log is append-only)
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
        && !field(r, "repo").to_lowercase().contains(&repo.to_lowercase())
    {
        return false;
    }
    if args.blocked && r.get("blocked").and_then(Value::as_bool) != Some(true) {
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

/// The recorded cost of a record/run JSON object, formatted for display — `$?` when absent, so a
/// missing cost can't read as genuine zero spend.
fn cost(v: &Value) -> String {
    crate::log::record_cost(v).map_or_else(|| "$?".to_owned(), crate::log::cost_display)
}

/// One log record as a compact row — tolerant of older records that predate some fields.
fn row(r: &Value, now: u64) -> String {
    let id = r.get("id").and_then(Value::as_str).unwrap_or("-");
    // A missing `ts` is surfaced as "?", not shown as a bogus age computed from 0 — matching how the
    // other fields (and stored_human's absolute time) disclose an absent value rather than faking one.
    let age = r
        .get("ts")
        .and_then(Value::as_u64)
        .map_or_else(|| "?".to_owned(), |ts| age(now.saturating_sub(ts)));
    let repo_full = field(r, "repo");
    let repo = repo_full
        .rsplit(['/', '\\'])
        .next()
        .expect("rsplit always yields at least one piece");
    let blocked = r.get("blocked").and_then(Value::as_bool).unwrap_or(false);
    format!(
        "{id} · {age} · {} · {} · {} · {}{}",
        field(r, "command"),
        repo,
        field(r, "model"),
        cost(r),
        if blocked { " · BLOCKED" } else { "" },
    )
}

/// A coarse relative age: seconds, minutes, hours, or days.
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
/// dependency, and labeled so it can't be misread as local time.
fn datetime_utc(secs: u64) -> String {
    let (y, m, d) = civil_from_days(i64::try_from(secs / SECS_PER_DAY).expect("fits until year ~25e12"));
    let rem = secs % SECS_PER_DAY;
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        rem / SECS_PER_HOUR,
        (rem % SECS_PER_HOUR) / SECS_PER_MINUTE
    )
}

fn show(id: &str, global: &GlobalArgs) -> anyhow::Result<()> {
    let id = resolve_id(id)?;
    let path = crate::log::result_path(&id).context("cannot determine the result path")?;
    let Some(text) = crate::read_optional(&path)
        .with_context(|| format!("cannot read {}", path.display()))?
    else {
        bail!(
            "no stored result for run `{id}` (runs predating the store, or made with logging off, aren't kept)"
        )
    };
    let stored: Value = serde_json::from_str(&text)
        .with_context(|| format!("invalid result file {}", path.display()))?;
    emit(&stored, &stored_human(&stored), global.json)
}

/// Resolve a full run id, or a unique prefix of one, against the result store (exact match wins;
/// an ambiguous prefix errors listing the candidates). An id with no stored entry passes through
/// unchanged so [`show`] reports the authoritative "no stored result" error.
fn resolve_id(prefix: &str) -> anyhow::Result<String> {
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

/// A stored run for humans: identity, then the run's parameters, then the result body (structured
/// if present, else text).
fn stored_human(v: &Value) -> String {
    let run = v.get("run").cloned().unwrap_or(Value::Null);
    let when = run
        .get("ts")
        .and_then(Value::as_u64)
        .map_or_else(|| "?".to_owned(), datetime_utc);
    let mut meta = format!("{} · {} · arc v{}", field(&run, "id"), when, field(&run, "version"));
    meta.push_str(&format!(
        "\n{} · {} · {}",
        field(&run, "command"),
        field(&run, "repo"),
        field(&run, "model")
    ));
    let runs = run.get("runs").and_then(Value::as_u64).unwrap_or(1) as usize;
    if runs > 1 {
        meta.push_str(&format!(" · runs={runs}"));
    }
    let budget = crate::log::budget_display(run.get("max_budget_usd").and_then(Value::as_f64), runs);
    meta.push_str(&format!(
        " · memory={} · budget={budget} · {}",
        field(&run, "memory"),
        cost(&run)
    ));
    if run.get("gate").and_then(Value::as_str).is_some() {
        let blocked = run.get("blocked").and_then(Value::as_bool) == Some(true);
        meta.push_str(if blocked { " · gate: BLOCKED" } else { " · gate: passed" });
    }
    let body = crate::synth::body_display(
        v.get("structured"),
        v.get("text").and_then(Value::as_str).unwrap_or(""),
    );
    format!("{meta}\n\n{body}")
}
