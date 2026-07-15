use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use crate::cli::{GlobalArgs, UsageArgs};
use crate::log::{SECS_PER_DAY, SECS_PER_HOUR, cost_display, field};
use crate::output::emit;

/// One aggregation window over the run log.
#[derive(Serialize)]
pub(crate) struct Window {
    pub(crate) window: &'static str,
    pub(crate) runs: usize,
    pub(crate) blocked: usize,
    /// Runs that errored — spent (their usage is in the token/cost sums) but didn't complete.
    pub(crate) errored: usize,
    pub(crate) cost_usd: f64,
    pub(crate) input_tokens: u64,
    pub(crate) cache_creation_input_tokens: u64,
    pub(crate) cache_read_input_tokens: u64,
    pub(crate) output_tokens: u64,
}

/// Per-command all-time totals.
#[derive(Serialize)]
pub(crate) struct CommandTotal {
    pub(crate) command: String,
    pub(crate) runs: usize,
    pub(crate) cost_usd: f64,
}

/// The full run-log rollup — the structured payload `--json` serializes and the TUI usage view renders
/// directly, so the CLI and TUI share one shape instead of the view re-parsing untyped JSON.
#[derive(Serialize)]
pub(crate) struct Rollup {
    pub(crate) windows: Vec<Window>,
    pub(crate) by_command: Vec<CommandTotal>,
    /// Disclosure lines (codex/missing/unparsed), preformatted so the CLI and TUI share their wording.
    pub(crate) notes: Vec<String>,
    pub(crate) tokens_only: usize,
    pub(crate) no_usage: usize,
    /// Runs whose spend is *unknown* (the backend returned no usage; recorded zeros are
    /// placeholders) — counted apart from the measured sums, never read as genuine zero.
    pub(crate) spend_unknown: usize,
    /// Runs from a cost-reporting backend whose record lacks a dollar cost — a lost cost, counted
    /// apart from the by-design tokens-only (codex) runs so the cost sums' under-count is disclosed.
    pub(crate) cost_missing: usize,
    /// Present-but-non-numeric usage fields encountered across records (each read as 0) —
    /// disclosed, so a mangled record can't masquerade as real zero consumption.
    pub(crate) malformed_fields: usize,
    pub(crate) no_timestamp: usize,
    pub(crate) unparsed: usize,
}

/// The `usage` command: a deterministic rollup of the run log — no AI, just the recorded ground
/// truth summed per window.
pub fn run(_args: &UsageArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    let (rollup, human) = rollup()?;
    emit(&serde_json::to_value(&rollup)?, &human, global.json)
}

/// Compute the run-log rollup once, returning the typed [`Rollup`] (serialized for `--json`, and
/// rendered directly by the TUI usage view) and the joined human-readable lines — one shape, so the
/// CLI and TUI can't drift.
pub(crate) fn rollup() -> anyhow::Result<(Rollup, String)> {
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
    // tokens but no dollar cost) contribute to token sums but not cost. Unknown-spend runs (the
    // backend returned no usage — zeros are placeholders) and malformed token fields are counted
    // apart, so neither reads as genuine zero. All are surfaced.
    let mut no_usage = 0usize;
    let mut tokens_only = 0usize;
    let mut spend_unknown = 0usize;
    let mut malformed_fields = 0usize;
    let mut cost_missing = 0usize;
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
                // An unknown-spend run's recorded numbers are a *lower bound* (zeros for a fully
                // unknown single run; the successful members' real tokens for a mixed fan-out).
                // Those known tokens still sum — discarding them would under-count real spend —
                // while the record itself is counted into its own disclosure and kept out of the
                // tokens-only (codex) note it would otherwise masquerade in.
                let unknown = crate::log::record_spend_unknown(r);
                if unknown && span.is_none() {
                    spend_unknown += 1;
                }
                let t = crate::log::usage_tokens(usage);
                if span.is_none() {
                    malformed_fields += t.malformed;
                }
                w.input_tokens += t.input;
                w.cache_creation_input_tokens += t.cache_creation;
                w.cache_read_input_tokens += t.cache_read;
                w.output_tokens += t.output;
                // Cost is summed only when present; a costless run (codex) is counted separately so the
                // cost figure's partialness is disclosed rather than silently read as $0.
                match crate::log::record_cost(r) {
                    Some(cost) => w.cost_usd += cost,
                    None => {
                        if span.is_none() && !unknown {
                            // Tokens-only *by design* (the backend reports no dollar cost) is the
                            // benign codex case; a cost-reporting backend's record with no cost
                            // LOST one — the cost sums under-count, disclosed apart. The registry
                            // owns which backend is which (an unknown backend counts as lost —
                            // can't-tell must not read as by-design).
                            let by_design = crate::ai::backend(&crate::log::field(r, "backend"))
                                .is_ok_and(|b| !b.reports_cost());
                            if by_design {
                                tokens_only += 1;
                            } else {
                                cost_missing += 1;
                            }
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
                    // The rollup's tokens-only/unknown-cost caveat is its own note below, sized by
                    // run count — not the per-line lower-bound marker.
                    false,
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
    if spend_unknown > 0 {
        notes.push(format!(
            "{spend_unknown} run(s) include unknown spend (the backend returned no usage for the run or a fan-out member) — their contributions to the sums are lower bounds, not measurements"
        ));
    }
    if cost_missing > 0 {
        notes.push(format!(
            "{cost_missing} run(s) from a cost-reporting backend lack a recorded dollar cost — the cost sums under-count"
        ));
    }
    if malformed_fields > 0 {
        notes.push(format!(
            "{malformed_fields} usage field(s) were absent or non-numeric (read as 0) — the sums may under-count"
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
    let rollup = Rollup {
        windows,
        by_command,
        notes,
        tokens_only,
        no_usage,
        spend_unknown,
        cost_missing,
        malformed_fields,
        no_timestamp,
        unparsed,
    };
    Ok((rollup, lines.join("\n")))
}
