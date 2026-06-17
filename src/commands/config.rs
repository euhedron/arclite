use anyhow::Context;
use serde::Serialize;

use crate::cli::{ConfigAction, ConfigArgs, GlobalArgs};
use crate::output::emit;
use crate::settings::Settings;

/// One settable scalar default: its dotted key, how to read its resolved value, and how a raw `set`
/// value is validated and typed into the JSON to store. `get`, `set`, `list`, and validation all
/// derive from this table; a new setting is one row here plus its typed field on
/// [`Settings`]/`RawDefaults` (settings.rs), which own the load/merge side. (Rulesets are
/// structured source-lists, not scalars, so they stay hand-edited or scaffolded.)
struct Setting {
    key: &'static str,
    read: fn(&Settings) -> Option<String>,
    parse: fn(&str) -> anyhow::Result<serde_json::Value>,
}

/// `parse` for the plain-string settings.
fn parse_string(value: &str) -> anyhow::Result<serde_json::Value> {
    Ok(serde_json::Value::String(value.to_owned()))
}

const SETTINGS: &[Setting] = &[
    Setting {
        key: "defaults.model",
        read: |s| s.default_model.clone(),
        parse: parse_string,
    },
    Setting {
        key: "defaults.backend",
        read: |s| s.default_backend.clone(),
        parse: |v| {
            crate::ai::validate_backend(v)?;
            Ok(serde_json::Value::String(v.to_owned()))
        },
    },
    Setting {
        key: "defaults.ruleset",
        read: |s| s.default_ruleset.clone(),
        parse: parse_string,
    },
    Setting {
        key: "defaults.logging",
        read: |s| Some(s.logging_enabled().to_string()),
        parse: |v| {
            Ok(serde_json::Value::Bool(
                v.parse::<bool>().context("expected `true` or `false`")?,
            ))
        },
    },
    Setting {
        key: "defaults.max_budget_usd",
        read: |s| s.default_max_budget_usd.map(|v| v.to_string()),
        parse: |v| {
            let cap: f64 = v.parse().context("expected a dollar amount")?;
            crate::settings::validate_budget(cap)?;
            Ok(serde_json::Value::from(cap))
        },
    },
    Setting {
        key: "defaults.codex_model",
        read: |s| s.default_codex_model.clone(),
        parse: parse_string,
    },
    Setting {
        key: "defaults.codex_reasoning_effort",
        read: |s| s.default_codex_reasoning_effort.clone(),
        parse: |v| {
            crate::ai::validate_reasoning_effort(v)?;
            Ok(serde_json::Value::String(v.to_owned()))
        },
    },
];

/// Look up a settable key, or error listing the known set — so `get` and `set` validate one way.
fn setting(key: &str) -> anyhow::Result<&'static Setting> {
    SETTINGS.iter().find(|s| s.key == key).ok_or_else(|| {
        anyhow::anyhow!(
            "unknown setting `{key}` (known: {})",
            SETTINGS
                .iter()
                .map(|s| s.key)
                .collect::<Vec<_>>()
                .join(", ")
        )
    })
}

/// The `config` command.
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

/// The resolved settings as (key, value) pairs (a `None` value = unset) plus the active layers — the
/// projection `arc config list` and the TUI's config view share, so the two can't drift apart.
pub(crate) struct ResolvedSettings {
    pub values: Vec<(&'static str, Option<String>)>,
    pub layers: Vec<String>,
}

/// Load and project `repo`'s settings: every settable key's resolved value (after user-then-project
/// layering) and the active layer paths.
pub(crate) fn resolved(repo: &std::path::Path) -> anyhow::Result<ResolvedSettings> {
    let s = Settings::load(repo)?;
    Ok(ResolvedSettings {
        values: SETTINGS.iter().map(|st| (st.key, (st.read)(&s))).collect(),
        layers: s.active_display(),
    })
}

/// Show every default's resolved value (after user-then-project layering) and the active layers.
fn list(global: &GlobalArgs) -> anyhow::Result<()> {
    let ResolvedSettings { values, layers } = resolved(std::path::Path::new("."))?;
    let mut lines: Vec<String> = values
        .iter()
        .map(|(key, value)| {
            format!(
                "{key}: {}",
                value
                    .clone()
                    .unwrap_or_else(|| crate::settings::UNSET.to_owned())
            )
        })
        .collect();
    lines.push(format!(
        "layers: {}",
        if layers.is_empty() {
            crate::settings::NO_LAYERS.to_owned()
        } else {
            layers.join(", ")
        }
    ));
    let settings = values
        .iter()
        .map(|(key, value)| ((*key).to_owned(), serde_json::json!(value)))
        .collect();
    emit(&Listed { settings, layers }, &lines.join("\n"), global.json)
}

/// Print one setting's resolved value.
fn get(key: &str, global: &GlobalArgs) -> anyhow::Result<()> {
    let s = Settings::load(std::path::Path::new("."))?;
    let value = (setting(key)?.read)(&s);
    let human = value
        .clone()
        .unwrap_or_else(|| crate::settings::UNSET.to_owned());
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
    let mut root: serde_json::Value =
        match crate::read_optional(&path).with_context(|| crate::settings::read_error(&path))? {
            Some(text) => {
                serde_json::from_str(&text).with_context(|| crate::settings::parse_error(&path))?
            }
            None => serde_json::json!({}),
        };
    let typed =
        (setting.parse)(value).with_context(|| format!("invalid value `{value}` for `{key}`"))?;
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
