//! Layered `.arc/settings.json` configuration: command defaults and named, composable rulesets,
//! merged from the user (`~/.arc/`) then the project (`<repo>/.arc/`). Project overrides user for
//! defaults; rulesets union, project winning on id collision. Because rules are *levers*, a ruleset
//! composes **sources** (directories or files of Markdown rules — possibly shared pools), not a
//! single directory — that's what lets "team core + this repo + my own" coexist.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Deserialize;

/// Merged settings. Ruleset source paths are resolved to absolute at load time.
#[derive(Debug, Default)]
pub struct Settings {
    pub default_model: Option<String>,
    pub default_backend: Option<String>,
    pub default_ruleset: Option<String>,
    pub default_logging: Option<bool>,
    pub default_max_budget_usd: Option<f64>,
    /// The settings files actually loaded, in layer order (user then project).
    pub active: Vec<PathBuf>,
    rulesets: BTreeMap<String, Vec<PathBuf>>,
}

#[derive(Debug, Default, Deserialize)]
struct Raw {
    #[serde(default)]
    defaults: RawDefaults,
    #[serde(default)]
    rulesets: BTreeMap<String, RawRuleset>,
}

/// Scalar command defaults as written in `settings.json`. Each key is a typed field here (+ a merge
/// arm below) and one row in the settable-key table in `commands/config.rs`.
#[derive(Debug, Default, Deserialize)]
struct RawDefaults {
    model: Option<String>,
    backend: Option<String>,
    ruleset: Option<String>,
    logging: Option<bool>,
    max_budget_usd: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
struct RawRuleset {
    #[serde(default)]
    sources: Vec<String>,
}

impl Settings {
    /// The user-layer settings file, `~/.arc/settings.json` (`None` if the home dir is unknown).
    pub fn user_path() -> Option<PathBuf> {
        Some(crate::arc_home()?.join(crate::SETTINGS_FILE))
    }

    /// The project-layer settings file for `repo`: `<repo>/.arc/settings.json`.
    pub fn project_path(repo: &Path) -> PathBuf {
        repo.join(crate::ARC_DIR).join(crate::SETTINGS_FILE)
    }

    /// Load + merge `~/.arc/settings.json` then `<repo>/.arc/settings.json`. A missing layer is
    /// fine; a present-but-unreadable or malformed file is a hard error.
    pub fn load(repo: &Path) -> anyhow::Result<Self> {
        let mut settings = Settings::default();
        if let Some(path) = Self::user_path() {
            settings.merge(&path)?;
        }
        settings.merge(&Self::project_path(repo))?;
        Ok(settings)
    }

    fn merge(&mut self, path: &Path) -> anyhow::Result<()> {
        // A missing file is fine — this layer is optional.
        let Some(text) = crate::read_optional(path).with_context(|| read_error(path))?
        else {
            return Ok(());
        };
        let raw: Raw = serde_json::from_str(&text).with_context(|| parse_error(path))?;
        self.active.push(path.to_path_buf());
        let dir = path
            .parent()
            .expect("a .arc/settings.json path always has a parent");
        overlay(&mut self.default_model, raw.defaults.model);
        overlay(&mut self.default_backend, raw.defaults.backend);
        overlay(&mut self.default_ruleset, raw.defaults.ruleset);
        overlay(&mut self.default_logging, raw.defaults.logging);
        // Validate a hand-edited cap on load too — `arc config set` checks it, but a malformed value
        // typed straight into settings.json would otherwise silently disable the safety lever.
        if let Some(cap) = raw.defaults.max_budget_usd {
            validate_budget(cap)
                .with_context(|| format!("invalid defaults.max_budget_usd in {}", path.display()))?;
        }
        overlay(&mut self.default_max_budget_usd, raw.defaults.max_budget_usd);
        for (id, rs) in raw.rulesets {
            let sources = rs.sources.iter().map(|s| resolve(dir, s)).collect();
            self.rulesets.insert(id, sources); // project (merged last) wins on id collision
        }
        Ok(())
    }

    /// The resolved source paths for ruleset `id`, if it is defined.
    pub fn ruleset(&self, id: &str) -> Option<&[PathBuf]> {
        self.rulesets.get(id).map(Vec::as_slice)
    }

    /// Whether per-run logging is on: the default, unless explicitly disabled. Single source for the
    /// "on unless `defaults.logging = false`" rule that `run_synthesis` gates on and `config` reports.
    pub fn logging_enabled(&self) -> bool {
        self.default_logging != Some(false)
    }

    /// The active settings-file layers as absolute-path display strings (user then project) — the
    /// projection the run report and `arc config list` share (`arc rules` shows them repo-relative).
    pub fn active_display(&self) -> Vec<String> {
        self.active.iter().map(|p| p.display().to_string()).collect()
    }
}

/// Overlay one default from a later settings layer: a set value wins, an unset one leaves the
/// earlier layer's in place — the layering rule, stated once for every scalar default.
fn overlay<T>(slot: &mut Option<T>, value: Option<T>) {
    if value.is_some() {
        *slot = value;
    }
}

/// The error context for a settings file that can't be read, and one that doesn't parse — one
/// wording each, shared by the loader (`merge`) and `arc config set` so they can't drift apart.
pub(crate) fn read_error(path: &Path) -> String {
    format!("cannot read settings file {}", path.display())
}
pub(crate) fn parse_error(path: &Path) -> String {
    format!("invalid settings file {}", path.display())
}

/// The validity rule for a budget cap — a positive, finite dollar amount — stated once for both
/// `arc config set` and settings load, so a hand-edited bad value can't silently neuter the cap.
pub(crate) fn validate_budget(cap: f64) -> anyhow::Result<()> {
    anyhow::ensure!(
        cap.is_finite() && cap > 0.0,
        "the budget cap must be a positive dollar amount"
    );
    Ok(())
}

/// The line shown when no `.arc/settings.json` layer is active — one wording for the run report,
/// `arc config list`, and `arc rules` (the empty-layers case had drifted across the three).
pub(crate) const NO_LAYERS: &str = "built-in defaults (no .arc/settings.json active)";

/// Resolve a ruleset source via the shared [`crate::resolve_path`] rule — relative sources are
/// relative to the settings file's own directory `dir` (so a repo's ruleset referencing `rules`
/// means *its* `.arc/rules`).
fn resolve(dir: &Path, src: &str) -> PathBuf {
    crate::resolve_path(dir, Path::new(src))
}
