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

#[derive(Debug, Default, Deserialize)]
struct RawDefaults {
    model: Option<String>,
    ruleset: Option<String>,
    logging: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct RawRuleset {
    #[serde(default)]
    sources: Vec<String>,
}

impl Settings {
    /// Load + merge `~/.arc/settings.json` then `<repo>/.arc/settings.json`. A missing layer is
    /// fine; a present-but-unreadable or malformed file is a hard error.
    pub fn load(repo: &Path) -> anyhow::Result<Self> {
        let mut settings = Settings::default();
        let relative = Path::new(crate::ARC_DIR).join("settings.json");
        if let Some(home) = dirs::home_dir() {
            settings.merge(&home.join(&relative))?;
        }
        settings.merge(&repo.join(&relative))?;
        Ok(settings)
    }

    fn merge(&mut self, path: &Path) -> anyhow::Result<()> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            // A missing file is fine — this layer is optional.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e).with_context(|| format!("cannot read settings file {}", path.display())),
        };
        let raw: Raw = serde_json::from_str(&text)
            .with_context(|| format!("invalid settings file {}", path.display()))?;
        self.active.push(path.to_path_buf());
        let dir = path
            .parent()
            .expect("a .arc/settings.json path always has a parent");
        if raw.defaults.model.is_some() {
            self.default_model = raw.defaults.model;
        }
        if raw.defaults.ruleset.is_some() {
            self.default_ruleset = raw.defaults.ruleset;
        }
        if raw.defaults.logging.is_some() {
            self.default_logging = raw.defaults.logging;
        }
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
}

/// Resolve a ruleset source: `~/…` → home; absolute → as-is; relative → relative to the settings
/// file's own directory `dir` (so a repo's ruleset referencing `rules` means *its* `.arc/rules`).
fn resolve(dir: &Path, src: &str) -> PathBuf {
    if let Some(rest) = src.strip_prefix("~/").or_else(|| src.strip_prefix("~\\"))
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    let p = Path::new(src);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        dir.join(p)
    }
}
