use crate::cli::{GlobalArgs, StatusArgs};
use crate::output::emit;

/// Report the runs currently in flight (the active-run registry).
pub fn run(_args: &StatusArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    let (active, unreadable) = crate::runs::active()?;
    let now = crate::log::now_secs();
    let mut lines = Vec::new();
    if active.is_empty() {
        lines.push("no active runs".to_owned());
    } else {
        lines.push(format!("{} active run(s):", active.len()));
        for r in &active {
            lines.push(format!(
                "  {} · {} · {} · {}s · pid {}",
                r.command,
                r.repo,
                r.model,
                now.saturating_sub(r.started_at),
                r.pid
            ));
        }
    }
    // Surface entries we couldn't read or parse, rather than under-reporting in-flight runs.
    if !unreadable.is_empty() {
        lines.push(format!(
            "{} unreadable registry entr{} skipped:",
            unreadable.len(),
            if unreadable.len() == 1 { "y" } else { "ies" }
        ));
        for path in &unreadable {
            lines.push(format!("  {}", path.display()));
        }
    }
    let human = lines.join("\n");
    let payload = serde_json::json!({
        "active": &active,
        "unreadable": unreadable.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
    });
    emit(&payload, &human, global.json)
}
