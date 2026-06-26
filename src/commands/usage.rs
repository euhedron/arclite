use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use crate::cli::{GlobalArgs, UsageArgs};
use crate::log::{SECS_PER_DAY, SECS_PER_HOUR, cost_display, field};
use crate::output::emit;

/// One aggregation window over the run log.
#[derive(Serialize)]
struct Window {
    window: &'static str,
    runs: usize,
    blocked: usize,
    /// Runs that errored — spent (their usage is in the token/cost sums) but didn't complete.
    errored: usize,
    cost_usd: f64,
    input_tokens: u64,
    cache_creation_input_tokens: u64,
    cache_read_input_tokens: u64,
    output_tokens: u64,
}

/// Per-command all-time totals.
#[derive(Serialize)]
struct CommandTotal {
    command: String,
    runs: usize,
    cost_usd: f64,
}

/// The `usage` command: a deterministic rollup of the run log — no AI, just the recorded ground
/// truth summed per window.
pub fn run(_args: &UsageArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    let (payload, human) = rollup()?;
    emit(&payload, &human, global.json)
}

/// Compute the run-log rollup once, returning the structured payload (for `--json`) and the joined
/// human-readable lines — shared by this command and the TUI usage view so the two can't drift.
pub(crate) fn rollup() -> anyhow::Result<(Value, String)> {
    let (records, unparsed) = crate::log::records()?;
    let now = crate::log::now_secs();
    // Each window is a label plus its maximum age; `None` = all time.
    let spans: [(&'static str, Option<u64>); 4] = [
        ("hour", Some(SECS_PER_HOUR)),
        ("day", Some(SECS_PER_DAY)),
        ("week", Some(7 * SECS_PER_DAY)),
        ("total", None),
    ];
    // Records with no `usage` object at all can't contribute to any sum; costless records (codex:
    // tokens but no dollar cost) contribute to token sums but not cost. Both are counted + surfaced.
    let mut no_usage = 0usize;
    let mut tokens_only = 0usize;
    // A record with no timestamp can't be placed in a finite window (it lands in the all-time total
    // only); count it so the windowed sums' omission is disclosed rather than silent.
    let no_timestamp = records
        .iter()
        .filter(|r| r.get("ts").and_then(Value::as_u64).is_none())
        .count();
    let windows: Vec<Window> = spans
        .iter()
        .map(|(label, span)| {
            let mut w = Window {
                window: label,
                runs: 0,
                blocked: 0,
                errored: 0,
                cost_usd: 0.0,
                input_tokens: 0,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
                output_tokens: 0,
            };
            for r in &records {
                // No `ts` → epoch 0, older than any finite window, so the record lands in the
                // all-time total only; the count is surfaced below as `no_timestamp`.
                let ts = r.get("ts").and_then(Value::as_u64).unwrap_or(0);
                if span.is_some_and(|s| now.saturating_sub(ts) > s) {
                    continue;
                }
                w.runs += 1;
                if crate::log::is_blocked(r) {
                    w.blocked += 1;
                }
                if crate::log::is_errored(r) {
                    w.errored += 1;
                }
                // Sum tokens for any record carrying a `usage` object — claude *and* codex. Codex
                // reports tokens without a dollar cost, so keying the token sums off cost (as before)
                // dropped codex usage entirely; only a record with no usage at all is excluded here.
                let Some(usage) = r.get("usage") else {
                    if span.is_none() {
                        no_usage += 1; // no usage object — counted once, on the all-time pass
                    }
                    continue;
                };
                let t = crate::log::usage_tokens(usage);
                w.input_tokens += t.input;
                w.cache_creation_input_tokens += t.cache_creation;
                w.cache_read_input_tokens += t.cache_read;
                w.output_tokens += t.output;
                // Cost is summed only when present; a costless run (codex) is counted separately so the
                // cost figure's partialness is disclosed rather than silently read as $0.
                match crate::log::record_cost(r) {
                    Some(cost) => w.cost_usd += cost,
                    None => {
                        if span.is_none() {
                            tokens_only += 1;
                        }
                    }
                }
            }
            w
        })
        .collect();

    // All-time per-command totals, for "where is the spend going".
    let mut by_command: BTreeMap<String, CommandTotal> = BTreeMap::new();
    for r in &records {
        let command = field(r, "command");
        let entry = by_command
            .entry(command.clone())
            .or_insert_with(|| CommandTotal {
                command,
                runs: 0,
                cost_usd: 0.0,
            });
        entry.runs += 1;
        if let Some(cost) = crate::log::record_cost(r) {
            entry.cost_usd += cost; // a costless record still counts as a run; no_usage discloses it
        }
    }
    let by_command: Vec<CommandTotal> = by_command.into_values().collect();

    let mut lines: Vec<String> = windows
        .iter()
        .map(|w| {
            format!(
                "{}: {} runs ({} blocked, {} errored) · {}",
                w.window,
                w.runs,
                w.blocked,
                w.errored,
                crate::log::usage_display(
                    w.input_tokens,
                    w.cache_creation_input_tokens,
                    w.cache_read_input_tokens,
                    w.output_tokens,
                    Some(w.cost_usd),
                ),
            )
        })
        .collect();
    if !by_command.is_empty() {
        lines.push("by command (total):".to_owned());
        for c in &by_command {
            lines.push(format!(
                "  {}: {} runs | {}",
                c.command,
                c.runs,
                cost_display(c.cost_usd)
            ));
        }
    }
    // Disclosure lines (codex token-only runs, missing usage/timestamps, unparsed) — built once here
    // and carried in the payload, so the TUI usage view renders the same wording rather than
    // re-deriving (and drifting from) it.
    let mut notes: Vec<String> = Vec::new();
    if tokens_only > 0 {
        notes.push(format!(
            "{tokens_only} run(s) report tokens only — no dollar cost (codex); counted in the token sums, not the cost"
        ));
    }
    if no_usage > 0 {
        notes.push(format!(
            "{no_usage} run(s) lack usage data entirely (excluded from all sums)"
        ));
    }
    if no_timestamp > 0 {
        notes.push(format!(
            "{no_timestamp} run(s) without a timestamp (in the all-time total only, not the timed windows)"
        ));
    }
    if unparsed > 0 {
        notes.push(crate::log::unparsed_note(unparsed));
    }
    lines.extend(notes.iter().cloned());
    let payload = serde_json::json!({
        "windows": windows,
        "by_command": by_command,
        "tokens_only": tokens_only,
        "no_usage": no_usage,
        "no_timestamp": no_timestamp,
        "unparsed": unparsed,
        "notes": notes,
    });
    Ok((payload, lines.join("\n")))
}
