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

/// The `usage` command: deterministic analytics over the run ledger — no AI, just recorded ground
/// truth. Two lenses over one dataset: the spend/volume rollup (default) and the per-rule firing
/// rollup (`--rules`); `--repo` filters either, because spend and firing are the same kind of
/// fact — a learning system's performance measured against a resource — and share their filters.
pub fn run(args: &UsageArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    if args.rules {
        let (rollup, human) = rules_rollup(
            args.repo.as_deref(),
            current_lens(std::path::Path::new(".")),
        )?;
        return emit(&serde_json::to_value(&rollup)?, &human, global.json);
    }
    let (rollup, human) = rollup(args.repo.as_deref())?;
    emit(&serde_json::to_value(&rollup)?, &human, global.json)
}

/// One rule *version's* exercise record. `hash: None` is the pre-record cohort: findings from runs
/// that predate the structured `rules` field, whose exposure is unknown — carried apart, never as
/// zero, and never back-parsed from the old records' display prose.
#[derive(Serialize)]
pub(crate) struct RuleVersionStat {
    pub(crate) hash: Option<String>,
    /// Whether this version's body is the one currently resolved by the lens directory's ruleset —
    /// the "what has the *active* rule done" split; `false` for historical versions, the
    /// pre-record cohort, and whenever no lens resolved.
    pub(crate) current: bool,
    /// Findings citing the rule in runs that exposed this version.
    pub(crate) fires: usize,
    /// Runs among those with at least one such finding — the recurrence count, distinct from a
    /// burst of findings inside one run.
    pub(crate) fired_runs: usize,
    /// Audit runs whose record lists this (id, hash) active. Zero for the pre-record cohort —
    /// its exposure is unknown, which the `None` hash marks.
    pub(crate) exposures: usize,
    pub(crate) last_fired_ts: Option<u64>,
}

/// One rule id's stats across its versions (current first, then by fires, pre-record last).
#[derive(Serialize)]
pub(crate) struct RuleStat {
    pub(crate) id: String,
    pub(crate) versions: Vec<RuleVersionStat>,
}

/// The per-rule firing rollup — deterministic ground truth from the run log joined to stored audit
/// findings. Signal for curation (sharpen, split, retire, generalize), never a verdict: firing is
/// not badness and silence is not uselessness — the display's job is to make the distribution and
/// its outliers visible.
#[derive(Serialize)]
pub(crate) struct RulesRollup {
    /// The repo filter applied (substring, case-insensitive), echoed; `None` = every repo.
    pub(crate) repo: Option<String>,
    pub(crate) audit_runs: usize,
    pub(crate) exposure_recorded_runs: usize,
    /// Runs that predate the structured `rules` field — their exposure is unknown, not zero.
    pub(crate) exposure_unknown_runs: usize,
    /// Runs whose stored result was missing or unreadable — their findings are uncounted, disclosed.
    pub(crate) results_unreadable: usize,
    pub(crate) rules: Vec<RuleStat>,
    pub(crate) notes: Vec<String>,
}

/// The currency lens for `repo`: the rules its ruleset resolves to right now, as (id, fingerprint)
/// pairs — what "current version" means for the stats. `None` when resolution fails (a missing or
/// unreadable layer); stats still compute, just without currency marking, and the rollup's notes
/// say so.
pub(crate) fn current_lens(repo: &std::path::Path) -> Option<Vec<crate::rules::ActiveRule>> {
    let settings = crate::settings::Settings::load(repo).ok()?;
    let resolution = super::resolve_rule_sources(None, None, &settings).ok()?;
    let (loaded, _, _) = crate::rules::load_sources(&resolution.sources).ok()?;
    let (active, _) = crate::rules::partition_disabled(loaded, &settings.disabled_rules);
    Some(
        active
            .into_iter()
            .map(|r| crate::rules::ActiveRule {
                hash: crate::rules::fingerprint(&r.body),
                id: r.id,
            })
            .collect(),
    )
}

/// The distinct repo paths the ledger has seen, sorted — the TUI's selectable lens set. Best-effort:
/// an unreadable ledger yields no lenses beyond the defaults, and the views it feeds surface their
/// own load errors.
pub(crate) fn ledger_repos() -> Vec<String> {
    crate::log::records()
        .map(|(records, _)| {
            let set: std::collections::BTreeSet<String> = records
                .iter()
                .map(|r| field(r, "repo"))
                .filter(|s| !s.is_empty())
                .collect();
            set.into_iter().collect()
        })
        .unwrap_or_default()
}

/// One version's human line — shared by the CLI rollup text and the TUI rules detail, so the two
/// renderings can't drift.
pub(crate) fn version_line(v: &RuleVersionStat, now: u64) -> String {
    let last = v
        .last_fired_ts
        .map(|ts| crate::commands::log::age(now.saturating_sub(ts)));
    match &v.hash {
        Some(h) => format!(
            "@{}{}: fired in {} of {} exposed run(s) · {} finding(s){}",
            &h[..8.min(h.len())],
            if v.current { " (current)" } else { "" },
            v.fired_runs,
            v.exposures,
            v.fires,
            last.map(|l| format!(" · last {l}")).unwrap_or_default()
        ),
        None => format!(
            "@pre-record: {} finding(s) in {} run(s) · exposure unknown{}",
            v.fires,
            v.fired_runs,
            last.map(|l| format!(" · last {l}")).unwrap_or_default()
        ),
    }
}

/// Compute the per-rule firing rollup: exposures from each audit record's structured `rules`
/// field, fires from its stored findings (each citing a rule id), joined per run so a finding
/// attributes to the rule *version* that was in play. Shared by `arc usage --rules` and the TUI.
pub(crate) fn rules_rollup(
    repo: Option<&str>,
    current: Option<Vec<crate::rules::ActiveRule>>,
) -> anyhow::Result<(RulesRollup, String)> {
    let (records, _unparsed) = crate::log::records()?;
    let now = crate::log::now_secs();
    #[derive(Default)]
    struct Agg {
        fires: usize,
        fired_runs: usize,
        exposures: usize,
        last_fired_ts: Option<u64>,
    }
    let mut by_version: BTreeMap<(String, Option<String>), Agg> = BTreeMap::new();
    let mut audit_runs = 0usize;
    let mut exposure_recorded_runs = 0usize;
    let mut exposure_unknown_runs = 0usize;
    let mut results_unreadable = 0usize;
    for r in &records {
        if field(r, "command") != crate::cli::NAME_AUDIT {
            continue;
        }
        if let Some(f) = repo
            && !crate::log::repo_matches(r, f)
        {
            continue;
        }
        audit_runs += 1;
        let ts = r.get("ts").and_then(Value::as_u64);
        // Exposure: the record's structured (id, hash) list. Absent = a pre-field run — unknown,
        // counted apart, never reconstructed from the record's display prose.
        let exposed: Option<BTreeMap<String, String>> =
            r.get("rules").and_then(Value::as_array).map(|pairs| {
                pairs
                    .iter()
                    .filter_map(|p| {
                        Some((
                            p.get("id")?.as_str()?.to_owned(),
                            p.get("hash")?.as_str()?.to_owned(),
                        ))
                    })
                    .collect()
            });
        match &exposed {
            Some(pairs) => {
                exposure_recorded_runs += 1;
                for (id, hash) in pairs {
                    by_version
                        .entry((id.clone(), Some(hash.clone())))
                        .or_default()
                        .exposures += 1;
                }
            }
            None => exposure_unknown_runs += 1,
        }
        // Fires: the stored result's findings, each citing its rule id; joined to the version this
        // run exposed. A missing/unreadable result leaves this run's findings uncounted — disclosed.
        // The id comes from an editable ledger line and is joined into a path, so it passes the same
        // guard as every other id→path boundary (log detail, resolve_id); an unsafe id lands in the
        // unreadable count rather than escaping the result store.
        let run_id = field(r, "id");
        if crate::commands::log::ensure_safe_run_id(&run_id).is_err() {
            results_unreadable += 1;
            continue;
        }
        let Some(path) = crate::log::result_path(&run_id) else {
            results_unreadable += 1;
            continue;
        };
        let findings: Vec<String> = match std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        {
            Some(v) => v
                .get("structured")
                .and_then(|s| s.get(crate::synth::RESULTS_KEY))
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|i| i.get("rule").and_then(Value::as_str))
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            None => {
                results_unreadable += 1;
                continue;
            }
        };
        let mut fired_versions: std::collections::BTreeSet<(String, Option<String>)> =
            std::collections::BTreeSet::new();
        for cited in findings {
            let version = exposed
                .as_ref()
                .and_then(|pairs| pairs.get(&cited).cloned());
            let key = (cited, version);
            let agg = by_version.entry(key.clone()).or_default();
            agg.fires += 1;
            agg.last_fired_ts = agg.last_fired_ts.max(ts);
            fired_versions.insert(key);
        }
        for key in fired_versions {
            by_version.entry(key).or_default().fired_runs += 1;
        }
    }
    // A currently-resolved rule with no recorded exercise still appears — zero exposures for its
    // version is a real (post-field) zero, and the never-fired outliers are half the point.
    if let Some(lens) = &current {
        for a in lens {
            by_version
                .entry((a.id.clone(), Some(a.hash.clone())))
                .or_default();
        }
    }
    let is_current = |id: &str, hash: &Option<String>| -> bool {
        match (&current, hash) {
            (Some(lens), Some(h)) => lens.iter().any(|a| a.id == id && &a.hash == h),
            _ => false,
        }
    };
    let mut by_rule: BTreeMap<String, Vec<RuleVersionStat>> = BTreeMap::new();
    for ((id, hash), agg) in by_version {
        let current_version = is_current(&id, &hash);
        by_rule
            .entry(id.clone())
            .or_default()
            .push(RuleVersionStat {
                current: current_version,
                hash,
                fires: agg.fires,
                fired_runs: agg.fired_runs,
                exposures: agg.exposures,
                last_fired_ts: agg.last_fired_ts,
            });
    }
    let mut rules: Vec<RuleStat> = by_rule
        .into_iter()
        .map(|(id, mut versions)| {
            // Current first, then most-fired, pre-record (hash: None) last.
            versions.sort_by(|a, b| {
                b.current
                    .cmp(&a.current)
                    .then(a.hash.is_none().cmp(&b.hash.is_none()))
                    .then(b.fired_runs.cmp(&a.fired_runs))
            });
            RuleStat { id, versions }
        })
        .collect();
    rules.sort_by(|a, b| {
        let fired = |s: &RuleStat| s.versions.iter().map(|v| v.fired_runs).sum::<usize>();
        fired(b).cmp(&fired(a)).then(a.id.cmp(&b.id))
    });

    let mut notes =
        vec!["fires are audit findings only — other verbs don't cite rule ids".to_owned()];
    if exposure_unknown_runs > 0 {
        notes.push(format!(
            "{exposure_unknown_runs} run(s) predate the structured rules field — their exposure is unknown (never zero) and their findings sit in the @pre-record bucket"
        ));
    }
    if results_unreadable > 0 {
        notes.push(format!(
            "{results_unreadable} run(s) have a missing/unreadable stored result — their findings are uncounted"
        ));
    }
    match &current {
        Some(_) => notes.push(
            "current = the version this directory's ruleset resolves to right now".to_owned(),
        ),
        None => notes.push(
            "no currency lens — this directory resolves no ruleset, so no version is marked current"
                .to_owned(),
        ),
    }

    // Human rendering: fired rules with their version breakdown, then the never-fired block —
    // the distribution's two tails, both visible.
    let mut lines = vec![match repo {
        Some(f) => format!(
            "rule firing · audit runs with repo ~ \"{f}\": {audit_runs} ({exposure_recorded_runs} exposure-recorded, {exposure_unknown_runs} pre-record)"
        ),
        None => format!(
            "rule firing · audit runs (all repos): {audit_runs} ({exposure_recorded_runs} exposure-recorded, {exposure_unknown_runs} pre-record)"
        ),
    }];
    let fired: Vec<&RuleStat> = rules
        .iter()
        .filter(|s| s.versions.iter().any(|v| v.fires > 0))
        .collect();
    if !fired.is_empty() {
        lines.push(format!("fired ({}):", fired.len()));
        for s in &fired {
            lines.push(format!("  {}", s.id));
            for v in &s.versions {
                if v.fires > 0 || v.current {
                    lines.push(format!("    {}", version_line(v, now)));
                }
            }
        }
    }
    let quiet: Vec<String> = rules
        .iter()
        .filter(|s| s.versions.iter().all(|v| v.fires == 0))
        .map(|s| {
            let exposures: usize = s.versions.iter().map(|v| v.exposures).sum();
            format!("  {} — {} exposed run(s)", s.id, exposures)
        })
        .collect();
    if !quiet.is_empty() {
        lines.push(format!("never fired ({}):", quiet.len()));
        lines.extend(quiet);
    }
    lines.extend(notes.iter().cloned());
    let rollup = RulesRollup {
        repo: repo.map(str::to_owned),
        audit_runs,
        exposure_recorded_runs,
        exposure_unknown_runs,
        results_unreadable,
        rules,
        notes,
    };
    Ok((rollup, lines.join("\n")))
}

/// Compute the run-log rollup once, returning the typed [`Rollup`] (serialized for `--json`, and
/// rendered directly by the TUI usage view) and the joined human-readable lines — one shape, so the
/// CLI and TUI can't drift. `repo` filters to runs whose repo path contains it (case-insensitive);
/// `None` = the whole ledger.
pub(crate) fn rollup(repo: Option<&str>) -> anyhow::Result<(Rollup, String)> {
    let (all_records, unparsed) = crate::log::records()?;
    let records: Vec<&Value> = all_records
        .iter()
        .filter(|r| match repo {
            Some(f) => crate::log::repo_matches(r, f),
            None => true,
        })
        .collect();
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
    if let Some(f) = repo {
        notes.push(format!(
            "filtered to runs whose repo path contains \"{f}\" ({} of {} runs)",
            records.len(),
            all_records.len()
        ));
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
