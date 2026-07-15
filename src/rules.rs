//! Rules: encoded "things that matter" — anti-patterns, standards, principles,
//! best practices, and the like — that a run can be weighed against.
//!
//! v1 is intentionally minimal: a rule is a **Markdown file**. Its filename stem
//! is the `id` (single source — no drift), and its contents are the body that
//! enters the AI's context. Frontmatter/attributes for selective inclusion
//! (kind, scope, tags, …) can be added later, once something actually filters on
//! them — not before.
//!
//! A source given explicitly as a **file** may also be a **JSON rule pack** — a
//! list of `{id, statement, enabled?}` objects (the shape rule-owning systems
//! keep as their single source of truth) — so arc audits against such a rulebook
//! directly instead of forcing a generated `.md` mirror that would drift.
//! Directory walks stay `.md`-only: a directory can hold unrelated JSON
//! (settings, reports); a pack must be named deliberately.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;

/// A single rule: an `id`, its `body`, and the file it was loaded from.
pub struct Rule {
    pub id: String,
    pub body: String,
    /// The file this rule came from — its provenance, surfaced by `arc rules`. Carried on the rule
    /// so the dedup in [`load_sources`] keeps the *winning* source when ids collide across sources.
    pub source: PathBuf,
}

/// Load `path` as a rule, or `None` if it isn't a `.md` file. A `.md` file whose stem can't be a
/// rule id (non-UTF-8) is a hard error, not a skip: a *present* rule silently vanishing from the
/// active set is exactly the kind of quiet shrinkage the loaders elsewhere refuse.
fn rule_from_file(path: &Path) -> anyhow::Result<Option<Rule>> {
    if path.extension().and_then(|e| e.to_str()) != Some("md") {
        return Ok(None);
    }
    let Some(id) = path.file_stem().and_then(|s| s.to_str()).map(str::to_owned) else {
        anyhow::bail!(
            "rule file {} has a non-UTF-8 name — its stem is the rule id, so rename it to load it",
            path.display()
        );
    };
    let body = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read rule {}", path.display()))?;
    Ok(Some(Rule {
        id,
        body: body.trim().to_owned(),
        source: path.to_owned(),
    }))
}

/// Load `path` as a JSON rule pack — a list of `{id, statement, enabled?}` objects — or `None`
/// if it isn't a `.json` file. A `.json` file that doesn't parse as a pack is a hard error, not
/// a skip: an explicitly named source silently contributing nothing is the same quiet shrinkage
/// `load_sources` refuses for typo'd paths. Entries with `enabled: false` are dropped HERE (the
/// pack's own switch, honored at its source); arc's `disabled_rules` still applies on top.
fn rules_from_json(path: &Path) -> anyhow::Result<Option<Vec<Rule>>> {
    if path.extension().and_then(|e| e.to_str()) != Some("json") {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read rule pack {}", path.display()))?;
    let entries: Vec<serde_json::Value> = serde_json::from_str(&text)
        .with_context(|| format!("rule pack {} is not a JSON list", path.display()))?;
    let mut rules = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        let id = e
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or_default();
        let body = e
            .get("statement")
            .or_else(|| e.get("body"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or_default();
        if id.is_empty() || body.is_empty() {
            anyhow::bail!(
                "rule pack {} entry {i} needs non-empty `id` and `statement`",
                path.display()
            );
        }
        if e.get("enabled").and_then(|v| v.as_bool()) == Some(false) {
            continue;
        }
        rules.push(Rule {
            id: id.to_owned(),
            body: body.to_owned(),
            source: path.to_owned(),
        });
    }
    Ok(Some(rules))
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

/// One id-collision [`load_sources`] resolved: the rule body that was replaced (its id and losing
/// source file) and the source that won — so the deliberate later-source-wins override is a
/// disclosed event, never a silent drop of an active rule body.
pub struct Overridden {
    pub id: String,
    pub replaced: PathBuf,
    pub winner: PathBuf,
}

/// Load rules from multiple sources (each a directory of `.md` files, a single `.md` file, or a
/// single `.json` rule pack), deduped by id with later sources winning — so a project ruleset
/// can override a shared pool's rule of the same id. Returns the loaded rules, any sources that
/// resolved to none of those forms — a typo'd or absent path the caller surfaces rather than
/// dropping silently, so a misconfigured source can't shrink the active ruleset unnoticed — and
/// every id collision the later-wins dedup resolved, for the same reason: the override is by
/// design, its silence wouldn't be.
/// (A present-but-unreadable source — or an absent `*.md` path — surfaces its I/O error, via
/// `try_is_dir`/`rule_from_file`, rather than being miscounted as a skipped typo.)
pub fn load_sources(
    sources: &[PathBuf],
) -> anyhow::Result<(Vec<Rule>, Vec<PathBuf>, Vec<Overridden>)> {
    let mut by_id: BTreeMap<String, Rule> = BTreeMap::new();
    let mut skipped = Vec::new();
    let mut overridden = Vec::new();
    let mut insert = |rule: Rule, overridden: &mut Vec<Overridden>| {
        if let Some(prev) = by_id.insert(rule.id.clone(), rule) {
            let winner = by_id[&prev.id].source.clone();
            overridden.push(Overridden {
                id: prev.id,
                replaced: prev.source,
                winner,
            });
        }
    };
    for src in sources {
        if crate::try_is_dir(src)
            .with_context(|| format!("cannot access rule source {}", src.display()))?
        {
            for rule in load(src)? {
                insert(rule, &mut overridden);
            }
        } else if let Some(rule) = rule_from_file(src)? {
            insert(rule, &mut overridden);
        } else if let Some(pack) = rules_from_json(src)? {
            for rule in pack {
                insert(rule, &mut overridden);
            }
        } else {
            skipped.push(src.clone());
        }
    }
    Ok((by_id.into_values().collect(), skipped, overridden))
}

/// Split `rules` into (active, disabled) by the configured disabled-id list, preserving order — the
/// one statement of rule disabling, shared by the rules report and the synthesis context, so a rule
/// can't be filtered on one surface yet slip into another.
pub fn partition_disabled(rules: Vec<Rule>, disabled: &[String]) -> (Vec<Rule>, Vec<Rule>) {
    rules.into_iter().partition(|r| !disabled.contains(&r.id))
}

/// Render rules as a prompt block — one section per rule (not a one-line bullet) so
/// multi-paragraph rule bodies round-trip when fed back via `--rules`.
pub fn render(rules: &[Rule]) -> String {
    rules
        .iter()
        .map(|rule| format!("## {}\n{}", rule.id, rule.body))
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch file under a per-test temp dir (std-only — no tempfile dependency).
    fn scratch(name: &str, contents: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("arclite-rules-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn json_pack_loads_and_honors_its_own_enabled_switch() {
        let pack = scratch(
            "pack.json",
            r#"[{"id": "a", "statement": "always A", "enabled": true},
                {"id": "b", "statement": "never B", "enabled": false},
                {"id": "c", "statement": "always C"}]"#,
        );
        let (rules, skipped, overridden) = load_sources(&[pack]).unwrap();
        assert_eq!(
            rules.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["a", "c"],
            "enabled:false entries are dropped at the pack, absent enabled defaults on"
        );
        assert_eq!(rules[0].body, "always A");
        assert!(skipped.is_empty() && overridden.is_empty());
    }

    #[test]
    fn json_pack_that_is_not_a_pack_is_an_error_not_a_skip() {
        let bogus = scratch("settings.json", r#"{"defaults": {"backend": "claude"}}"#);
        assert!(
            load_sources(&[bogus]).is_err(),
            "an explicitly named JSON source that isn't a rule pack must fail loudly"
        );
    }

    #[test]
    fn json_pack_entry_missing_fields_is_an_error() {
        let pack = scratch("partial.json", r#"[{"id": "only-id"}]"#);
        assert!(
            load_sources(&[pack]).is_err(),
            "a pack entry without a statement is a hard error — partial packs are a rule-owning system's overlay concept, not arc's"
        );
    }

    #[test]
    fn md_and_pack_dedup_later_source_wins() {
        let pack = scratch(
            "dup-pack.json",
            r#"[{"id": "dup", "statement": "from pack"}]"#,
        );
        let md = scratch("dup.md", "from md");
        let (rules, _, overridden) = load_sources(&[pack.clone(), md.clone()]).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].body, "from md", "later source wins");
        assert_eq!(overridden.len(), 1);
        assert_eq!(overridden[0].replaced, pack);
    }

    #[test]
    fn directory_walk_stays_md_only() {
        let pack = scratch(
            "walked.json",
            r#"[{"id": "w", "statement": "should not load"}]"#,
        );
        let dir = pack.parent().unwrap().to_path_buf();
        let (rules, _, _) = load_sources(&[dir]).unwrap();
        assert!(
            !rules.iter().any(|r| r.id == "w"),
            "a directory source must not pick up JSON — packs are named deliberately"
        );
    }
}
