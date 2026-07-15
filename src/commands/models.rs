use crate::cli::{GlobalArgs, ModelsArgs};
use crate::output::emit;

/// The `models` command: each backend's provider-reported model listing — the authoritative
/// enumeration (neither agent CLI lists models headlessly, the providers' APIs do). Per backend it
/// reports the models, the key's provenance (whose account the list reflects), and any pagination
/// truncation; a backend without a key reports *why* and both ways to supply one, never an empty
/// list that reads as "no models exist".
pub fn run(args: &ModelsArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    let settings = crate::settings::Settings::load(std::path::Path::new("."))?;
    let backends = match &args.backend {
        Some(name) => {
            crate::ai::validate_backend(name)?;
            vec![name.as_str()]
        }
        None => crate::ai::known_backends(),
    };

    let mut lines: Vec<String> = Vec::new();
    let mut payload = serde_json::Map::new();
    for name in backends {
        let backend = crate::ai::backend(name)?;
        match backend.list_models(&settings) {
            Ok(listing) => {
                lines.push(format!(
                    "{name} · {} model(s) · key: {}",
                    listing.models.len(),
                    listing.key_source
                ));
                for m in &listing.models {
                    match &m.display_name {
                        Some(display) => lines.push(format!("  {}  ({display})", m.id)),
                        None => lines.push(format!("  {}", m.id)),
                    }
                }
                if listing.truncated {
                    lines.push("  … the provider reports more pages exist".to_owned());
                }
                if listing.undated > 0 {
                    lines.push(format!(
                        "  note: {} model(s) carry no `created` timestamp — sorted last, not as oldest",
                        listing.undated
                    ));
                }
                payload.insert(
                    name.to_owned(),
                    serde_json::json!({
                        "models": listing.models.iter().map(|m| serde_json::json!({
                            "id": m.id,
                            "display_name": m.display_name,
                        })).collect::<Vec<_>>(),
                        "key_source": listing.key_source,
                        "truncated": listing.truncated,
                        "undated": listing.undated,
                    }),
                );
            }
            Err(e) => {
                lines.push(format!("{name} · {e:#}"));
                payload.insert(
                    name.to_owned(),
                    serde_json::json!({ "error": format!("{e:#}") }),
                );
            }
        }
    }
    emit(
        &serde_json::Value::Object(payload),
        &lines.join("\n"),
        global.json,
    )
}
