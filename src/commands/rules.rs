use std::path::Path;

use crate::cli::{GlobalArgs, RulesArgs};
use crate::output::emit;

/// One resolved rule as the report carries it: `id`, display-ready provenance, the Markdown body, and
/// whether the settings' disabled list switches it off (kept in the report — a disabled rule must stay
/// visible to be re-enabled, and its absence from runs disclosed).
pub(crate) struct RuleEntry {
    pub id: String,
    pub source: String,
    pub body: String,
    pub disabled: bool,
}

/// The resolved-rules projection — the one statement backing `arc rules` (payload + human) and the
/// TUI's rules view, so the surfaces can't drift. Paths are display-ready: relative to the repo root
/// where possible (project rules), else absolute (e.g. a user-level shared pool), so the provenance
/// stays readable.
pub(crate) struct Report {
    pub description: String,
    pub sources: Vec<String>,
    pub rules: Vec<RuleEntry>,
    /// Configured disabled ids that match no resolved rule — stale entries, surfaced so they can be
    /// cleaned up rather than silently rotting in settings.
    pub disabled_unmatched: Vec<String>,
    pub skipped: Vec<String>,
    /// Id collisions the later-source-wins dedup resolved (`id: replaced → winner`, display-ready) —
    /// the override is by design, but each occurrence is disclosed, never silent.
    pub overridden: Vec<String>,
    pub layers: Vec<String>,
}

/// Resolve the active ruleset for `path` into a [`Report`]. `rules_override`/`ruleset` mirror the
/// CLI flags (an ad-hoc source overriding a named ruleset overriding the configured default).
pub(crate) fn resolved(
    path: &Path,
    rules_override: Option<&Path>,
    ruleset: Option<&str>,
) -> anyhow::Result<Report> {
    let root = super::resolve_root(path)?;
    let settings = crate::settings::Settings::load(path)?;
    let resolution = super::resolve_rule_sources(rules_override, ruleset, &settings)?;
    let (rules, skipped, overridden) = crate::rules::load_sources(&resolution.sources)?;
    let rel = |p: &Path| p.strip_prefix(&root).unwrap_or(p).display().to_string();
    // The disabling judgment comes from the one statement of it — the same partition synthesis
    // filters with — never a reimplemented membership check that could drift from what runs actually
    // exclude. The report shows both halves (a disabled rule must stay visible to be re-enabled),
    // re-merged in id order; a configured id that disabled nothing is stale, surfaced as unmatched.
    let (active, disabled) = crate::rules::partition_disabled(rules, &settings.disabled_rules);
    let disabled_unmatched = settings
        .disabled_rules
        .iter()
        .filter(|id| !disabled.iter().any(|r| &r.id == *id))
        .cloned()
        .collect();
    let mut entries: Vec<RuleEntry> = active
        .into_iter()
        .map(|r| (r, false))
        .chain(disabled.into_iter().map(|r| (r, true)))
        .map(|(r, disabled)| RuleEntry {
            disabled,
            id: r.id,
            source: rel(&r.source),
            body: r.body,
        })
        .collect();
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(Report {
        description: resolution.description.clone(),
        sources: resolution.sources.iter().map(|p| rel(p)).collect(),
        rules: entries,
        disabled_unmatched,
        skipped: skipped.iter().map(|p| rel(p)).collect(),
        overridden: overridden
            .iter()
            .map(|o| format!("{}: {} → {}", o.id, rel(&o.replaced), rel(&o.winner)))
            .collect(),
        layers: settings.active.iter().map(|p| rel(p)).collect(),
    })
}

/// The `rules` command — beyond the rules themselves, it also surfaces disabled rules, skipped
/// sources, and the settings layers in effect.
pub fn run(args: &RulesArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    let report = resolved(&args.path, args.rules.as_deref(), args.ruleset.as_deref())?;

    let mut lines = vec![report.description.clone()];
    if report.sources.is_empty() {
        lines.push("sources: (none)".to_owned());
    } else {
        lines.push("sources:".to_owned());
        for s in &report.sources {
            lines.push(format!("  {s}"));
        }
    }
    let disabled = report.rules.iter().filter(|r| r.disabled).count();
    if disabled == 0 {
        lines.push(format!("rules ({}):", report.rules.len()));
    } else {
        lines.push(format!(
            "rules ({} active, {disabled} disabled):",
            report.rules.len() - disabled,
        ));
    }
    for r in &report.rules {
        if r.disabled {
            lines.push(format!("  {} ← {}  (disabled)", r.id, r.source));
        } else {
            lines.push(format!("  {} ← {}", r.id, r.source));
        }
    }
    if !report.disabled_unmatched.is_empty() {
        lines.push(format!(
            "disabled ids matching no rule ({}): {}",
            report.disabled_unmatched.len(),
            report.disabled_unmatched.join(", ")
        ));
    }
    if !report.skipped.is_empty() {
        lines.push(format!("skipped sources ({}):", report.skipped.len()));
        for s in &report.skipped {
            lines.push(format!("  {s}"));
        }
    }
    if !report.overridden.is_empty() {
        lines.push(format!(
            "overridden by a later source ({}):",
            report.overridden.len()
        ));
        for o in &report.overridden {
            lines.push(format!("  {o}"));
        }
    }
    lines.push(format!(
        "settings: {}",
        crate::join_or(&report.layers, crate::settings::NO_LAYERS)
    ));

    let payload = serde_json::json!({
        "selection": report.description,
        "sources": report.sources,
        "rules": report.rules
            .iter()
            .map(|r| serde_json::json!({ "id": r.id, "source": r.source, "disabled": r.disabled }))
            .collect::<Vec<_>>(),
        "disabled_unmatched": report.disabled_unmatched,
        "skipped": report.skipped,
        "overridden": report.overridden,
        "settings": report.layers,
    });
    emit(&payload, &lines.join("\n"), global.json)
}
