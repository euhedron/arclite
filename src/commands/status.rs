use crate::cli::{GlobalArgs, StatusArgs};
use crate::output::emit;

/// The `status` command.
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
                "  {} · {} · {} · {} · {} turns · {} tools · {} chars · pid {} #{}",
                r.command,
                r.repo,
                r.model,
                r.age_display(now),
                r.turns,
                r.tool_calls,
                r.output_chars,
                r.pid,
                r.index
            ));
        }
    }
    // Surface entries we couldn't read or parse, rather than under-reporting in-flight runs.
    if !unreadable.is_empty() {
        lines.push(format!(
            "{} skipped:",
            crate::runs::unreadable_entries(unreadable.len())
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
