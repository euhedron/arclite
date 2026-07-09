use std::path::Path;

use crate::cli::{GlobalArgs, RulesArgs};
use crate::output::emit;

/// The `rules` command — beyond the rules themselves, it also surfaces skipped sources and the
/// settings layers in effect.
pub fn run(args: &RulesArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    let (payload, human) = report(&args.path, args.rules.as_deref(), args.ruleset.as_deref())?;
    emit(&payload, &human, global.json)
}

/// The resolved-rules report for `path` — the JSON payload and human text `arc rules` emits, shared
/// with the TUI's rules view so the two can't drift. `rules_override`/`ruleset` mirror the CLI flags.
pub(crate) fn report(
    path: &Path,
    rules_override: Option<&Path>,
    ruleset: Option<&str>,
) -> anyhow::Result<(serde_json::Value, String)> {
    let root = super::resolve_root(path)?;
    let settings = crate::settings::Settings::load(path)?;
    let resolution = super::resolve_rule_sources(rules_override, ruleset, &settings)?;
    let (rules, skipped) = crate::rules::load_sources(&resolution.sources)?;

    // Show paths relative to the repo root where possible (project rules), else absolute (e.g. a
    // user-level shared pool), so the provenance stays readable.
    let rel = |p: &Path| p.strip_prefix(&root).unwrap_or(p).display().to_string();

    let mut lines = vec![resolution.description.clone()];
    if resolution.sources.is_empty() {
        lines.push("sources: (none)".to_owned());
    } else {
        lines.push("sources:".to_owned());
        for s in &resolution.sources {
            lines.push(format!("  {}", rel(s)));
        }
    }
    lines.push(format!("rules ({}):", rules.len()));
    for r in &rules {
        lines.push(format!("  {} ← {}", r.id, rel(&r.source)));
    }
    if !skipped.is_empty() {
        lines.push(format!("skipped sources ({}):", skipped.len()));
        for s in &skipped {
            lines.push(format!("  {}", rel(s)));
        }
    }
    let layers: Vec<String> = settings.active.iter().map(|p| rel(p)).collect();
    lines.push(format!(
        "settings: {}",
        crate::join_or(&layers, crate::settings::NO_LAYERS)
    ));

    let payload = serde_json::json!({
        "selection": resolution.description.clone(),
        "sources": resolution.sources.iter().map(|p| rel(p)).collect::<Vec<_>>(),
        "rules": rules
            .iter()
            .map(|r| serde_json::json!({ "id": r.id.clone(), "source": rel(&r.source) }))
            .collect::<Vec<_>>(),
        "skipped": skipped.iter().map(|p| rel(p)).collect::<Vec<_>>(),
        "settings": settings.active.iter().map(|p| rel(p)).collect::<Vec<_>>(),
    });
    Ok((payload, lines.join("\n")))
}
