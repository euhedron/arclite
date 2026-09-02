//! `arc repos` — the device map's CLI read: every repository the machine's agent activity and
//! arc's ledger know, by discovery (see `crate::discovery`), newest activity first, the
//! suppression posture and every store's degradations disclosed.

use crate::cli::{GlobalArgs, ReposArgs};
use crate::output::emit;

/// One row's cells — shared with the TUI status block so the two surfaces can't drift on how a
/// discovered repo reads.
pub(crate) fn row(r: &crate::discovery::RepoActivity, now: u64) -> String {
    format!(
        "{} · claude {} · codex {} · runs {} · {}",
        crate::display_path(&r.repo),
        r.claude_sessions,
        r.codex_sessions,
        r.ledger_runs,
        r.recency(now)
    )
}

/// The `repos` command.
pub fn run(args: &ReposArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    let now = crate::log::now_secs();
    let d = crate::discovery::discover(args.all);
    let mut lines = vec![format!(
        "repos ({} discovered{}{})",
        d.repos.len(),
        if d.muted > 0 {
            format!(" · {} muted", d.muted)
        } else {
            String::new()
        },
        if d.gone > 0 && !args.all {
            format!(" · {} gone from disk (arc repos --all shows all)", d.gone)
        } else if d.gone > 0 {
            format!(" · {} gone from disk", d.gone)
        } else {
            String::new()
        }
    )];
    for r in &d.repos {
        lines.push(format!("  {}", row(r, now)));
    }
    for note in &d.notes {
        lines.push(note.clone());
    }
    let payload = serde_json::json!({
        "repos": d.repos,
        "muted": d.muted,
        "gone": d.gone,
        "notes": d.notes,
    });
    emit(&payload, &lines.join("\n"), global.json)
}
