use anyhow::Context;
use serde::Serialize;

use crate::cli::{ConfigAction, ConfigArgs, GlobalArgs};
use crate::output::emit;
use crate::settings::Settings;

/// One settable scalar default: its dotted key, how to read its resolved value, and whether its
/// value is a boolean. The single source for the key set — `get`, `set`, `list`, and validation all
/// derive from this, so adding a setting is one row. (Rulesets are structured source-lists, not
/// scalars, so they stay hand-edited or scaffolded.)
struct Setting {
    key: &'static str,
    read: fn(&Settings) -> Option<String>,
    is_bool: bool,
}

const SETTINGS: &[Setting] = &[
    Setting {
        key: "defaults.model",
        read: |s| s.default_model.clone(),
        is_bool: false,
    },
    Setting {
        key: "defaults.ruleset",
        read: |s| s.default_ruleset.clone(),
        is_bool: false,
    },
    Setting {
        key: "defaults.logging",
        read: |s| Some(s.logging_enabled().to_string()),
        is_bool: true,
    },
];

/// Look up a settable key, or error listing the known set — so `get` and `set` validate one way.
fn setting(key: &str) -> anyhow::Result<&'static Setting> {
    SETTINGS.iter().find(|s| s.key == key).ok_or_else(|| {
        anyhow::anyhow!(
            "unknown setting `{key}` (known: {})",
            SETTINGS.iter().map(|s| s.key).collect::<Vec<_>>().join(", ")
        )
    })
}

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
struct Listed {
    settings: serde_json::Map<String, serde_json::Value>,
    layers: Vec<String>,
}

/// Show every default's resolved value (after user-then-project layering) and the active layers.
fn list(global: &GlobalArgs) -> anyhow::Result<()> {
    let s = Settings::load(std::path::Path::new("."))?;
    let layers: Vec<String> = s.active.iter().map(|p| p.display().to_string()).collect();
    let mut lines: Vec<String> = SETTINGS
        .iter()
        .map(|st| {
            format!(
                "{}: {}",
                st.key,
                (st.read)(&s).unwrap_or_else(|| "(unset)".to_owned())
            )
        })
        .collect();
    lines.push(format!(
        "layers: {}",
        if layers.is_empty() {
            "(none — built-in defaults)".to_owned()
        } else {
            layers.join(", ")
        }
    ));
    let settings = SETTINGS
        .iter()
        .map(|st| (st.key.to_owned(), serde_json::json!((st.read)(&s))))
        .collect();
    emit(&Listed { settings, layers }, &lines.join("\n"), global.json)
}

/// Print one setting's resolved value.
fn get(key: &str, global: &GlobalArgs) -> anyhow::Result<()> {
    let s = Settings::load(std::path::Path::new("."))?;
    let value = (setting(key)?.read)(&s);
    let human = value.clone().unwrap_or_else(|| "(unset)".to_owned());
    emit(
        &serde_json::json!({ "key": key, "value": value }),
        &human,
        global.json,
    )
}

/// Write one setting into a layer's `settings.json`, preserving every other key (and any rulesets).
fn set(key: &str, value: &str, user: bool, global: &GlobalArgs) -> anyhow::Result<()> {
    let setting = setting(key)?;
    let path = if user {
        Settings::user_path().context("cannot determine the home directory for the user layer")?
    } else {
        Settings::project_path(std::path::Path::new("."))
    };
    // Load the existing layer (or start fresh) as a Value, so unrelated keys round-trip untouched.
    let mut root: serde_json::Value = match crate::read_optional(&path)
        .with_context(|| format!("cannot read settings file {}", path.display()))?
    {
        Some(text) => serde_json::from_str(&text)
            .with_context(|| format!("invalid settings file {}", path.display()))?,
        None => serde_json::json!({}),
    };
    let typed = if setting.is_bool {
        serde_json::Value::Bool(
            value
                .parse::<bool>()
                .with_context(|| format!("`{key}` must be `true` or `false`, not `{value}`"))?,
        )
    } else {
        serde_json::Value::String(value.to_owned())
    };
    // Navigate (creating as needed) the dotted key path, e.g. `defaults.model`, and set the leaf.
    let parts: Vec<&str> = key.split('.').collect();
    let (leaf, parents) = parts.split_last().expect("a settable key is never empty");
    let mut node = &mut root;
    for part in parents {
        node = node
            .as_object_mut()
            .with_context(|| format!("`{part}` in {} is not a JSON object", path.display()))?
            .entry(*part)
            .or_insert_with(|| serde_json::json!({}));
    }
    node.as_object_mut()
        .with_context(|| format!("the root of {} is not a JSON object", path.display()))?
        .insert((*leaf).to_owned(), typed);
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
