use std::path::Path;

use crate::cli::{GlobalArgs, RulesArgs};
use crate::output::emit;

/// One resolved rule as the report carries it: `id`, display-ready provenance, and the Markdown body.
pub(crate) struct RuleEntry {
    pub id: String,
    pub source: String,
    pub body: String,
}

/// The resolved-rules projection — the one statement backing `arc rules` (payload + human) and the
/// TUI's rules view, so the surfaces can't drift. Paths are display-ready: relative to the repo root
/// where possible (project rules), else absolute (e.g. a user-level shared pool), so the provenance
/// stays readable.
pub(crate) struct Report {
    pub description: String,
    pub sources: Vec<String>,
    pub rules: Vec<RuleEntry>,
    pub skipped: Vec<String>,
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
    let (rules, skipped) = crate::rules::load_sources(&resolution.sources)?;
    let rel = |p: &Path| p.strip_prefix(&root).unwrap_or(p).display().to_string();
    Ok(Report {
        description: resolution.description.clone(),
        sources: resolution.sources.iter().map(|p| rel(p)).collect(),
        rules: rules
            .into_iter()
            .map(|r| RuleEntry {
                id: r.id,
                source: rel(&r.source),
                body: r.body,
            })
            .collect(),
        skipped: skipped.iter().map(|p| rel(p)).collect(),
        layers: settings.active.iter().map(|p| rel(p)).collect(),
    })
}

/// The `rules` command — beyond the rules themselves, it also surfaces skipped sources and the
/// settings layers in effect.
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
    lines.push(format!("rules ({}):", report.rules.len()));
    for r in &report.rules {
        lines.push(format!("  {} ← {}", r.id, r.source));
    }
    if !report.skipped.is_empty() {
        lines.push(format!("skipped sources ({}):", report.skipped.len()));
        for s in &report.skipped {
            lines.push(format!("  {s}"));
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
            .map(|r| serde_json::json!({ "id": r.id, "source": r.source }))
            .collect::<Vec<_>>(),
        "skipped": report.skipped,
        "settings": report.layers,
    });
    emit(&payload, &lines.join("\n"), global.json)
}
