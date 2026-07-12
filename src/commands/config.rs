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
    /// The key's value space: a closed enumerable set (the TUI picks among them, from the same
    /// single sources the validators use), a provider-fetched remote listing (the model keys), or
    /// open — a dollar amount, a comma-list, a secret — edited as free text.
    space: fn(&Settings) -> ValueSpace,
}

/// A settable key's value space — how an editor should offer values.
pub(crate) enum ValueSpace {
    /// Free text; any (validated) value.
    Open,
    /// A closed set: exactly these values are valid.
    Closed(Vec<String>),
    /// An open space whose current values the named backend's provider API lists live (model ids) —
    /// fetched on demand; free entry stays valid beyond the listing.
    Remote { backend: &'static str },
}

/// `space` for the open-valued settings (free text).
fn open_space(_: &Settings) -> ValueSpace {
    ValueSpace::Open
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
        space: |_| ValueSpace::Remote {
            backend: crate::ai::CLAUDE,
        },
    },
    Setting {
        key: "defaults.backend",
        read: |s| s.default_backend.clone(),
        parse: |v| {
            crate::ai::validate_backend(v)?;
            Ok(serde_json::Value::String(v.to_owned()))
        },
        space: |_| {
            ValueSpace::Closed(
                crate::ai::known_backends()
                    .iter()
                    .map(|b| (*b).to_owned())
                    .collect(),
            )
        },
    },
    Setting {
        key: "defaults.ruleset",
        read: |s| s.default_ruleset.clone(),
        parse: parse_string,
        // The defined rulesets are the meaningful values; none defined -> free text (a future one).
        space: |s| {
            let ids = s.ruleset_ids();
            if ids.is_empty() {
                ValueSpace::Open
            } else {
                ValueSpace::Closed(ids)
            }
        },
    },
    Setting {
        key: "defaults.logging",
        read: |s| Some(s.logging_enabled().to_string()),
        parse: |v| {
            Ok(serde_json::Value::Bool(
                v.parse::<bool>().context("expected `true` or `false`")?,
            ))
        },
        space: |_| ValueSpace::Closed(vec!["true".to_owned(), "false".to_owned()]),
    },
    Setting {
        key: "defaults.max_budget_usd",
        read: |s| s.default_max_budget_usd.map(|v| v.to_string()),
        parse: |v| {
            let cap: f64 = v.parse().context("expected a dollar amount")?;
            crate::settings::validate_budget(cap)?;
            Ok(serde_json::Value::from(cap))
        },
        space: open_space,
    },
    Setting {
        key: "defaults.codex_model",
        read: |s| s.default_codex_model.clone(),
        parse: parse_string,
        space: |_| ValueSpace::Remote {
            backend: crate::ai::CODEX,
        },
    },
    Setting {
        key: "defaults.codex_reasoning_effort",
        read: |s| s.default_codex_reasoning_effort.clone(),
        parse: |v| {
            crate::ai::validate_reasoning_effort(v)?;
            Ok(serde_json::Value::String(v.to_owned()))
        },
        space: |_| {
            ValueSpace::Closed(
                crate::ai::CODEX_REASONING_EFFORTS
                    .iter()
                    .map(|e| (*e).to_owned())
                    .collect(),
            )
        },
    },
    // A root-level key (a list beside `defaults`/`rulesets`, not a scalar default) — the dotted-path
    // writer below handles the single-segment path the same way. Read/written as a comma-joined line;
    // stored as a JSON array.
    Setting {
        key: "disabled_rules",
        read: |s| (!s.disabled_rules.is_empty()).then(|| s.disabled_rules.join(",")),
        parse: |v| {
            Ok(serde_json::Value::Array(
                v.split(',')
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                    .map(|id| serde_json::Value::String(id.to_owned()))
                    .collect(),
            ))
        },
        space: open_space,
    },
    // The api_keys rows: user-layer only (`set_value` elevates + the loader rejects a project-layer
    // key) and masked — list/get/the TUI show presence, never the secret. An empty value unsets.
    Setting {
        key: "api_keys.anthropic",
        read: |s| {
            s.api_key_anthropic
                .as_ref()
                .map(|_| crate::settings::SET_MASK.to_owned())
        },
        parse: parse_secret,
        space: open_space,
    },
    Setting {
        key: "api_keys.openai",
        read: |s| {
            s.api_key_openai
                .as_ref()
                .map(|_| crate::settings::SET_MASK.to_owned())
        },
        parse: parse_secret,
        space: open_space,
    },
];

/// `parse` for the secret settings: a single-line value stores as a string; an empty value parses to
/// JSON `null`, which [`set_value`] treats as "remove the key" — the unset path for secrets.
fn parse_secret(value: &str) -> anyhow::Result<serde_json::Value> {
    let v = value.trim();
    if v.is_empty() {
        return Ok(serde_json::Value::Null);
    }
    Ok(serde_json::Value::String(v.to_owned()))
}

/// Whether `key` may only live in the user layer — the secrets: a project's settings.json is
/// tracked, and a tracked file must never hold one.
pub(crate) fn user_layer_only(key: &str) -> bool {
    key.starts_with("api_keys.")
}

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

/// One resolved setting: its key, value (`None` = unset), and — when the key's value space is a
/// closed, enumerable set — the valid options the TUI's picker offers.
pub(crate) struct ResolvedSetting {
    pub key: &'static str,
    pub value: Option<String>,
    pub space: ValueSpace,
}

/// The resolved settings plus the active layers — the projection `arc config list` and the TUI's
/// config view share, so the two can't drift apart.
pub(crate) struct ResolvedSettings {
    pub values: Vec<ResolvedSetting>,
    pub layers: Vec<String>,
}

/// Load and project `repo`'s settings: every settable key's resolved value (after user-then-project
/// layering) and the active layer paths.
pub(crate) fn resolved(repo: &std::path::Path) -> anyhow::Result<ResolvedSettings> {
    let s = Settings::load(repo)?;
    Ok(ResolvedSettings {
        values: SETTINGS
            .iter()
            .map(|st| ResolvedSetting {
                key: st.key,
                value: (st.read)(&s),
                space: (st.space)(&s),
            })
            .collect(),
        layers: s.active_display(),
    })
}

/// Show every default's resolved value (after user-then-project layering) and the active layers.
fn list(global: &GlobalArgs) -> anyhow::Result<()> {
    let ResolvedSettings { values, layers } = resolved(std::path::Path::new("."))?;
    let mut lines: Vec<String> = values
        .iter()
        .map(|v| {
            format!(
                "{}: {}",
                v.key,
                v.value
                    .clone()
                    .unwrap_or_else(|| crate::settings::UNSET.to_owned())
            )
        })
        .collect();
    lines.push(format!(
        "layers: {}",
        crate::join_or(&layers, crate::settings::NO_LAYERS)
    ));
    let settings = values
        .iter()
        .map(|v| (v.key.to_owned(), serde_json::json!(v.value)))
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

/// Write one setting into a layer's `settings.json`, preserving every other key (and any rulesets) —
/// the shared write path behind `arc config set` and the TUI's editors. `repo` anchors the project
/// layer; returns the file written. The value is validated and typed by the key's [`Setting::parse`].
pub(crate) fn set_value(
    repo: &std::path::Path,
    key: &str,
    value: &str,
    user: bool,
) -> anyhow::Result<std::path::PathBuf> {
    let setting = setting(key)?;
    // Secrets are elevated to the user layer regardless of the flag: a project's settings.json is
    // tracked, and a tracked file must never hold one.
    let user = user || user_layer_only(key);
    let path = if user {
        Settings::user_path().context("cannot determine the home directory for the user layer")?
    } else {
        Settings::project_path(repo)
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
    let object = node
        .as_object_mut()
        .with_context(|| format!("the root of {} is not a JSON object", path.display()))?;
    if typed.is_null() {
        // A null from the parse means "unset" — remove the leaf (writing a literal null would read
        // as set-but-empty).
        object.remove(*leaf);
    } else {
        object.insert((*leaf).to_owned(), typed);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    std::fs::write(&path, format!("{}\n", serde_json::to_string_pretty(&root)?))
        .with_context(|| format!("cannot write {}", path.display()))?;
    Ok(path)
}

/// The `config set` CLI: write via [`set_value`] and report where.
fn set(key: &str, value: &str, user: bool, global: &GlobalArgs) -> anyhow::Result<()> {
    let path = set_value(std::path::Path::new("."), key, value, user)?;
    // A secret's value is never echoed — not to the terminal, not into a --json consumer's log.
    let shown = if user_layer_only(key) {
        if value.trim().is_empty() {
            crate::settings::UNSET
        } else {
            crate::settings::SET_MASK
        }
    } else {
        value
    };
    let human = format!("set {key} = {shown}  ({})", path.display());
    emit(
        &serde_json::json!({ "key": key, "value": shown, "path": path.display().to_string() }),
        &human,
        global.json,
    )
}
