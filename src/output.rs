use serde::Serialize;

/// arclite is agent-first: every command can emit machine-readable JSON
/// (`--json`) or human-readable text from the same underlying data.
pub fn emit<T: Serialize>(data: &T, human: &str, json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(data)?);
    } else {
        println!("{human}");
    }
    Ok(())
}
