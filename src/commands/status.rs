use crate::cli::{GlobalArgs, StatusArgs};
use crate::output::emit;

/// Report the runs currently in flight (the active-run registry). Deterministic — no LLM.
pub fn run(_args: &StatusArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    let active = crate::runs::active();
    let now = crate::log::now_secs();
    let human = if active.is_empty() {
        "no active runs".to_owned()
    } else {
        let mut lines = vec![format!("{} active run(s):", active.len())];
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
        lines.join("\n")
    };
    emit(&active, &human, global.json)
}
