//! `arc items` — the agenda's access surface: the open items in their intended order, the order's
//! integrity computed and disclosed, and any single item's body on request. One loader serves this
//! command, the TUI's items view, and synth's agenda gathering, so what a session reads, what a
//! run audits, and what the cockpit shows can't drift.

use std::path::Path;

use anyhow::Context;

use crate::cli::{GlobalArgs, ItemsArgs};
use crate::output::emit;

/// The loaded agenda: every open item, the intended order (if an order file exists), the order's
/// integrity verdicts, and the resolved-ledger count — computed once at load.
pub(crate) struct Agenda {
    /// Every open item (filename stem = id, body = the whole file), in the loader's id order.
    pub items: Vec<crate::rules::Rule>,
    /// The parsed order file; `None` when `.arc/items/order.json` is absent.
    pub order: Option<Vec<String>>,
    /// Ordered ids that resolve to no open item.
    pub dangling: Vec<String>,
    /// Ids listed more than once.
    pub duplicated: Vec<String>,
    /// Open items the order omits.
    pub unlisted: Vec<String>,
    /// Files in the resolved ledger — the surfaceable trail's size.
    pub resolved: usize,
}

impl Agenda {
    /// The one-line integrity verdict every surface shows — sources lines, `arc items`, the TUI.
    pub fn integrity(&self) -> String {
        match &self.order {
            None => "no order file".to_owned(),
            Some(_)
                if self.unlisted.is_empty()
                    && self.dangling.is_empty()
                    && self.duplicated.is_empty() =>
            {
                "order: complete".to_owned()
            }
            Some(_) => {
                let mut parts = Vec::new();
                if !self.unlisted.is_empty() {
                    parts.push(format!("{} open item(s) unlisted", self.unlisted.len()));
                }
                if !self.dangling.is_empty() {
                    parts.push(format!("{} id(s) resolve to no item", self.dangling.len()));
                }
                if !self.duplicated.is_empty() {
                    parts.push(format!("{} duplicated id(s)", self.duplicated.len()));
                }
                format!("order: {}", parts.join(", "))
            }
        }
    }

    /// The ids as presented everywhere: the intended order first (dangling ids included — they are
    /// the order file's content, and hiding them would mask the very drift the integrity line
    /// names), then any unlisted open items.
    pub fn presented_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.order.iter().flatten().cloned().collect();
        ids.extend(self.unlisted.iter().cloned());
        ids
    }

    /// One open item's body by id.
    pub fn body(&self, id: &str) -> Option<&str> {
        self.items
            .iter()
            .find(|i| i.id == id)
            .map(|i| i.body.as_str())
    }
}

/// Load a repo's agenda: the open ledger, the order file (malformed = a hard error, absent =
/// disclosed as such), the integrity verdicts, and the resolved count.
pub(crate) fn load(repo: &Path) -> anyhow::Result<Agenda> {
    let items = crate::synth::load_ledger_dir(&crate::items_open_dir(repo), "items")?;
    let order_path = crate::items_order_path(repo);
    let order: Option<Vec<String>> = match crate::read_optional(&order_path)
        .with_context(|| format!("cannot read {}", order_path.display()))?
    {
        Some(text) => {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Order {
                order: Vec<String>,
            }
            let parsed: Order = serde_json::from_str(&text)
                .with_context(|| format!("invalid order file {}", order_path.display()))?;
            Some(parsed.order)
        }
        None => None,
    };
    let ids: std::collections::BTreeSet<&str> = items.iter().map(|i| i.id.as_str()).collect();
    let mut listed: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    let mut dangling = Vec::new();
    let mut duplicated = Vec::new();
    for id in order.iter().flatten() {
        if !listed.insert(id.as_str()) {
            duplicated.push(id.clone());
        }
        if !ids.contains(id.as_str()) {
            dangling.push(id.clone());
        }
    }
    let unlisted: Vec<String> = items
        .iter()
        .map(|i| i.id.clone())
        .filter(|id| !listed.contains(id.as_str()))
        .collect();
    let resolved = count_md(&crate::items_resolved_dir(repo))?;
    Ok(Agenda {
        items,
        order,
        dangling,
        duplicated,
        unlisted,
        resolved,
    })
}

/// `.md` files in a ledger directory — absent is zero (an empty trail), unreadable is an error.
fn count_md(dir: &Path) -> anyhow::Result<usize> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => {
            return Err(anyhow::Error::new(e).context(format!("reading {}", dir.display())));
        }
    };
    let mut n = 0;
    for entry in entries {
        let entry = entry.with_context(|| format!("reading {}", dir.display()))?;
        if entry.file_name().to_string_lossy().ends_with(".md") {
            n += 1;
        }
    }
    Ok(n)
}

pub fn run(args: &ItemsArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    let agenda = load(&args.path)?;
    if let Some(id) = args.id.as_deref() {
        return show_one(&agenda, &args.path, id, global);
    }
    let mut lines = vec![format!(
        "items ({} open, {} resolved) · {}",
        agenda.items.len(),
        agenda.resolved,
        agenda.integrity()
    )];
    for (i, id) in agenda.presented_ids().iter().enumerate() {
        let marker = if agenda.dangling.iter().any(|d| d == id) {
            "  (dangling — no item file)"
        } else if agenda.unlisted.iter().any(|u| u == id) {
            "  (unlisted — not in order.json)"
        } else {
            ""
        };
        lines.push(format!("{:>3}. {id}{marker}", i + 1));
    }
    if agenda.items.is_empty() && agenda.order.is_none() {
        lines.push(
            "no agenda: .arc/items/open is absent or empty, and there is no order file".to_owned(),
        );
    }
    let payload = serde_json::json!({
        "open": agenda.items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
        "order": agenda.order,
        "unlisted": agenda.unlisted,
        "dangling": agenda.dangling,
        "duplicated": agenda.duplicated,
        "resolved": agenda.resolved,
    });
    emit(&payload, &lines.join("\n"), global.json)
}

/// Show one item in full: an open item's body, or — so the trail stays surfaceable without
/// reconstructing history — a resolved item's file when no open one matches.
fn show_one(agenda: &Agenda, repo: &Path, id: &str, global: &GlobalArgs) -> anyhow::Result<()> {
    let (status, body) = if let Some(body) = agenda.body(id) {
        // Served from the already-loaded agenda — no path is built from `id`, so every stem the
        // listing shows is showable here, whatever characters it carries.
        ("open", body.to_owned())
    } else {
        // Falling through to the resolved ledger joins `id` into a path, so only here must it be
        // a safe single segment (the same bar the run-result store applies to its ids).
        crate::commands::log::ensure_safe_run_id(id).map_err(|_| {
            anyhow::anyhow!("`{id}` is not a usable item id (a single path segment)")
        })?;
        let resolved_path = crate::items_resolved_dir(repo).join(format!("{id}.md"));
        match crate::read_optional(&resolved_path)
            .with_context(|| format!("cannot read {}", resolved_path.display()))?
        {
            Some(body) => ("resolved", body),
            None => anyhow::bail!(
                "no item `{id}` — open: {}",
                agenda
                    .items
                    .iter()
                    .map(|i| i.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    };
    let human = format!("{id} ({status})\n\n{}", body.trim_end());
    let payload = serde_json::json!({ "id": id, "status": status, "body": body });
    emit(&payload, &human, global.json)
}
