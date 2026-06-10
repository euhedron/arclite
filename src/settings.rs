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
        let Some(text) = crate::read_optional(path)
            .with_context(|| format!("cannot read settings file {}", path.display()))?
        else {
            return Ok(());
        };
        let raw: Raw = serde_json::from_str(&text)
            .with_context(|| format!("invalid settings file {}", path.display()))?;
        self.active.push(path.to_path_buf());
        let dir = path
            .parent()
            .expect("a .arc/settings.json path always has a parent");
        overlay(&mut self.default_model, raw.defaults.model);
        overlay(&mut self.default_ruleset, raw.defaults.ruleset);
        overlay(&mut self.default_logging, raw.defaults.logging);
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
}

/// Overlay one default from a later settings layer: a set value wins, an unset one leaves the
/// earlier layer's in place — the layering rule, stated once for every scalar default.
fn overlay<T>(slot: &mut Option<T>, value: Option<T>) {
    if value.is_some() {
        *slot = value;
    }
}

/// Resolve a ruleset source via the shared [`crate::resolve_path`] rule — relative sources are
/// relative to the settings file's own directory `dir` (so a repo's ruleset referencing `rules`
/// means *its* `.arc/rules`).
fn resolve(dir: &Path, src: &str) -> PathBuf {
    crate::resolve_path(dir, Path::new(src))
}
