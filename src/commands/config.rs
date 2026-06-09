use anyhow::{Context, bail};
use serde::Serialize;

use crate::cli::{ConfigAction, ConfigArgs, GlobalArgs};
use crate::output::emit;
use crate::settings::Settings;

/// The settable scalar defaults — validated so a typo'd key is rejected, never silently written.
/// (Rulesets are structured source-lists, not scalars, so they stay hand-edited or scaffolded.)
const KEY_MODEL: &str = "defaults.model";
const KEY_RULESET: &str = "defaults.ruleset";
const KEY_LOGGING: &str = "defaults.logging";
const KEYS: &[&str] = &[KEY_MODEL, KEY_RULESET, KEY_LOGGING];

/// Get, set, or list the scalar settings defaults — the current directory's project layer (and the
/// user layer).
pub fn run(args: &ConfigArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    match &args.action {
        ConfigAction::List => list(global),
        ConfigAction::Get { key } => get(key, global),
        ConfigAction::Set { key, value, user } => set(key, value, *user, global),
    }
}

#[derive(Serialize)]
struct Listed<'a> {
    model: Option<&'a str>,
    ruleset: Option<&'a str>,
    logging: bool,
    layers: Vec<String>,
}

/// Show the resolved defaults (after user-then-project layering) and which settings files are active.
fn list(global: &GlobalArgs) -> anyhow::Result<()> {
    let s = Settings::load(std::path::Path::new("."))?;
    let listed = Listed {
        model: s.default_model.as_deref(),
        ruleset: s.default_ruleset.as_deref(),
        logging: s.logging_enabled(),
        layers: s.active.iter().map(|p| p.display().to_string()).collect(),
    };
    let human = format!(
        "defaults.model: {}\ndefaults.ruleset: {}\ndefaults.logging: {}\nlayers: {}",
        listed.model.unwrap_or("(built-in default)"),
        listed.ruleset.unwrap_or("(none)"),
        listed.logging,
        if listed.layers.is_empty() {
            "(none — built-in defaults)".to_owned()
        } else {
            listed.layers.join(", ")
        },
    );
    emit(&listed, &human, global.json)
}

/// Print one setting's resolved value.
fn get(key: &str, global: &GlobalArgs) -> anyhow::Result<()> {
    let s = Settings::load(std::path::Path::new("."))?;
    let value = match key {
        KEY_MODEL => s.default_model.clone(),
        KEY_RULESET => s.default_ruleset.clone(),
        KEY_LOGGING => Some(s.logging_enabled().to_string()),
        _ => bail!("unknown setting `{key}` (known: {})", KEYS.join(", ")),
    };
    let human = value.clone().unwrap_or_else(|| "(unset)".to_owned());
    emit(
        &serde_json::json!({ "key": key, "value": value }),
        &human,
        global.json,
    )
}

/// Write one setting into a layer's `settings.json`, preserving every other key (and any rulesets).
fn set(key: &str, value: &str, user: bool, global: &GlobalArgs) -> anyhow::Result<()> {
    if !KEYS.contains(&key) {
        bail!("unknown setting `{key}` (known: {})", KEYS.join(", "));
    }
    let path = if user {
        crate::arc_home()
            .context("cannot determine the home directory for the user settings layer")?
            .join("settings.json")
    } else {
        std::path::Path::new(crate::ARC_DIR).join("settings.json")
    };
    // Load the existing layer (or start fresh) as a Value, so unrelated keys round-trip untouched.
    let mut root: serde_json::Value = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text)
            .with_context(|| format!("invalid settings file {}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(e) => {
            return Err(e).with_context(|| format!("cannot read settings file {}", path.display()));
        }
    };
    let sub = key
        .strip_prefix("defaults.")
        .expect("every KEY starts with `defaults.`");
    let typed = if key == KEY_LOGGING {
        serde_json::Value::Bool(
            value
                .parse::<bool>()
                .with_context(|| format!("`{key}` must be `true` or `false`, not `{value}`"))?,
        )
    } else {
        serde_json::Value::String(value.to_owned())
    };
    root.as_object_mut()
        .context("the settings file's root is not a JSON object")?
        .entry("defaults")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .context("`defaults` in the settings file is not a JSON object")?
        .insert(sub.to_owned(), typed);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    std::fs::write(&path, format!("{}\n", serde_json::to_string_pretty(&root)?))
        .with_context(|| format!("cannot write {}", path.display()))?;
    let human = format!("set {key} = {value}  ({})", path.display());
    emit(
        &serde_json::json!({ "key": key, "value": value, "path": path.display().to_string() }),
        &human,
        global.json,
    )
}
