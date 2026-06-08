//! Rules: encoded "things that matter" — anti-patterns, standards, principles,
//! best practices, and the like — that a run can be weighed against.
//!
//! v1 is intentionally minimal: a rule is a **Markdown file**. Its filename stem
//! is the `id` (single source — no drift), and its contents are the body that
//! enters the AI's context. Frontmatter/attributes for selective inclusion
//! (kind, scope, tags, …) can be added later, once something actually filters on
//! them — not before.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;

/// A single rule: an `id` and its `body`.
pub struct Rule {
    pub id: String,
    pub body: String,
}

/// Load `path` as a rule, or `None` if it isn't a `.md` file.
fn rule_from_file(path: &Path) -> anyhow::Result<Option<Rule>> {
    if path.extension().and_then(|e| e.to_str()) != Some("md") {
        return Ok(None);
    }
    let Some(id) = path.file_stem().and_then(|s| s.to_str()).map(str::to_owned) else {
        return Ok(None);
    };
    let body = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read rule {}", path.display()))?;
    Ok(Some(Rule {
        id,
        body: body.trim().to_owned(),
    }))
}

/// Load all `*.md` rules from `dir`.
pub fn load(dir: &Path) -> anyhow::Result<Vec<Rule>> {
    let mut rules = Vec::new();
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("cannot read rules dir {}", dir.display()))?;
    for entry in entries {
        if let Some(rule) = rule_from_file(&entry?.path())? {
            rules.push(rule);
        }
    }
    rules.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(rules)
}

/// Load rules from multiple sources (each a directory of `.md` files or a single `.md` file),
/// deduped by id with later sources winning — so a project ruleset can override a shared pool's
/// rule of the same id. Returns the loaded rules plus any sources that resolved to neither a
/// directory nor a `.md` file — a typo'd or absent path the caller surfaces rather than dropping
/// silently, so a misconfigured source can't shrink the active ruleset unnoticed.
/// (An absent `*.md` path is louder still: `rule_from_file` fails to read it and the error propagates.)
pub fn load_sources(sources: &[PathBuf]) -> anyhow::Result<(Vec<Rule>, Vec<PathBuf>)> {
    let mut by_id: BTreeMap<String, Rule> = BTreeMap::new();
    let mut skipped = Vec::new();
    for src in sources {
        if src.is_dir() {
            for rule in load(src)? {
                by_id.insert(rule.id.clone(), rule);
            }
        } else if let Some(rule) = rule_from_file(src)? {
            by_id.insert(rule.id.clone(), rule);
        } else {
            skipped.push(src.clone());
        }
    }
    Ok((by_id.into_values().collect(), skipped))
}

/// Render rules as a prompt block — one section per rule (not a one-line bullet) so
/// multi-paragraph bodies (e.g. the body + `_provenance:_` line that `extract` emits)
/// round-trip when fed back via `--rules`.
pub fn render(rules: &[Rule]) -> String {
    rules
        .iter()
        .map(|rule| format!("## {}\n{}", rule.id, rule.body))
        .collect::<Vec<_>>()
        .join("\n\n")
}
