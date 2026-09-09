use serde::Serialize;

/// Reject `--json` for a command whose output isn't JSON — one policy and one message shape for
/// every such command (a shell script, an interactive view), so a rejection can't drift or be
/// forgotten as commands join. `what` names the command and, parenthesized, what it emits instead.
pub fn reject_json(json: bool, what: &str) -> anyhow::Result<()> {
    anyhow::ensure!(!json, "`--json` has no meaning for {what}");
    Ok(())
}

/// Emit a command's result as pretty JSON (`--json`) or the human-readable text.
pub fn emit<T: Serialize>(data: &T, human: &str, json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(data)?);
    } else {
        println!("{human}");
    }
    Ok(())
}
