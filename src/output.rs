use serde::Serialize;

/// Emit a command's result as pretty JSON (`--json`) or the human-readable text.
pub fn emit<T: Serialize>(data: &T, human: &str, json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(data)?);
    } else {
        println!("{human}");
    }
    Ok(())
}
