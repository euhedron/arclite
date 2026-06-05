//! Rules: encoded "things that matter" — anti-patterns, standards, principles,
//! best practices, and the like — that a run can be weighed against.
//!
//! v1 is intentionally minimal: a rule is a **Markdown file**. Its filename stem
//! is the `id` (single source — no drift), and its contents are the body that
//! enters the AI's context. Frontmatter/attributes for selective inclusion
//! (kind, scope, tags, …) can be added later, once something actually filters on
//! them — not before.

use std::path::Path;

use anyhow::Context;

/// A single rule: `id` (the filename stem) identifies it; `body` is the text.
pub struct Rule {
    pub id: String,
    pub body: String,
}

/// Load all `*.md` rules from `dir` (filename stem = id, file contents = body).
pub fn load(dir: &Path) -> anyhow::Result<Vec<Rule>> {
    let mut rules = Vec::new();
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("cannot read rules dir {}", dir.display()))?;
    for entry in entries {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()).map(str::to_owned) else {
            continue;
        };
        let body = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read rule {}", path.display()))?;
        rules.push(Rule {
            id,
            body: body.trim().to_owned(),
        });
    }
    rules.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(rules)
}

/// Load rules from an optional directory and render them as a prompt block.
/// Returns `None` when no directory is given or it contains no rules.
pub fn block(dir: Option<&Path>) -> anyhow::Result<Option<String>> {
    let Some(dir) = dir else { return Ok(None) };
    let rules = load(dir)?;
    if rules.is_empty() {
        return Ok(None);
    }
    // One section per rule (not a one-line bullet) so multi-paragraph bodies — e.g. the
    // body + `_provenance:_` line that `extract` emits — round-trip when fed back via `--rules`.
    let rendered = rules
        .iter()
        .map(|rule| format!("## {}\n{}", rule.id, rule.body))
        .collect::<Vec<_>>()
        .join("\n\n");
    Ok(Some(rendered))
}
