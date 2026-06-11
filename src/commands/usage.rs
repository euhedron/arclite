use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use crate::cli::{GlobalArgs, UsageArgs};
use crate::log::{SECS_PER_DAY, SECS_PER_HOUR, cost_display};
use crate::output::emit;

/// One aggregation window over the run log.
#[derive(Serialize)]
struct Window {
    window: &'static str,
    runs: usize,
    blocked: usize,
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
    let (records, unparsed) = crate::log::records()?;
    let now = crate::log::now_secs();
    // Each window is a label plus its maximum age; `None` = all time.
    let spans: [(&'static str, Option<u64>); 4] = [
        ("hour", Some(SECS_PER_HOUR)),
        ("day", Some(SECS_PER_DAY)),
        ("week", Some(7 * SECS_PER_DAY)),
        ("total", None),
    ];
    // Records without usage data can't contribute to the sums — counted and surfaced, not dropped.
    let mut no_usage = 0usize;
    let windows: Vec<Window> = spans
        .iter()
        .map(|(label, span)| {
            let mut w = Window {
                window: label,
                runs: 0,
                blocked: 0,
                cost_usd: 0.0,
                input_tokens: 0,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
                output_tokens: 0,
            };
            for r in &records {
                let ts = r.get("ts").and_then(Value::as_u64).unwrap_or(0);
                if span.is_some_and(|s| now.saturating_sub(ts) > s) {
                    continue;
                }
                w.runs += 1;
                if r.get("blocked").and_then(Value::as_bool) == Some(true) {
                    w.blocked += 1;
                }
                let Some(cost) = crate::log::record_cost(r) else {
                    if span.is_none() {
                        no_usage += 1; // count once, on the all-time pass
                    }
                    continue;
                };
                w.cost_usd += cost;
                let usage = r.get("usage").expect("cost_usd was read from within usage");
                let n = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
                w.input_tokens += n("input_tokens");
                w.cache_creation_input_tokens += n("cache_creation_input_tokens");
                w.cache_read_input_tokens += n("cache_read_input_tokens");
                w.output_tokens += n("output_tokens");
            }
            w
        })
        .collect();

    // All-time per-command totals, for "where is the spend going".
    let mut by_command: BTreeMap<String, CommandTotal> = BTreeMap::new();
    for r in &records {
        let command = r
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_owned();
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
                "{}: {} runs ({} blocked) · {}",
                w.window,
                w.runs,
                w.blocked,
                crate::log::usage_display(
                    w.input_tokens,
                    w.cache_creation_input_tokens,
                    w.cache_read_input_tokens,
                    w.output_tokens,
                    w.cost_usd,
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
    if no_usage > 0 {
        lines.push(format!("{no_usage} run(s) lack usage data (excluded from the sums)"));
    }
    if unparsed > 0 {
        lines.push(format!("{unparsed} unparseable log line(s) skipped"));
    }
    let payload = serde_json::json!({
        "windows": windows,
        "by_command": by_command,
        "no_usage": no_usage,
        "unparsed": unparsed,
    });
    emit(&payload, &lines.join("\n"), global.json)
}
