//! Active-run registry: each real synthesis run writes a marker to `~/.arc/runs/<pid>.json` on
//! start and removes it on exit (via the returned guard), so `arc status` can list what's in flight.
//! Separate from the append-only completed-run log in `log.rs`; one file per process, so concurrent
//! runs don't contend.

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

/// One in-flight run, as recorded in the registry.
#[derive(Serialize, Deserialize)]
pub struct ActiveRun {
    pub pid: u32,
    pub command: String,
    pub repo: String,
    pub model: String,
    pub started_at: u64,
}

/// The registry directory, `~/.arc/runs/` (`None` if the home directory is unknown).
fn dir() -> Option<PathBuf> {
    Some(crate::arc_home()?.join("runs"))
}

/// Marker that removes its registry entry on drop, so a run clears itself on exit — success, error,
/// or unwind. Returned by [`register`]; hold it for the run's lifetime.
pub struct Registered(PathBuf);

impl Drop for Registered {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Record this process as an in-flight run; the returned guard clears it on drop. Best-effort:
/// returns `None` (the run proceeds untracked) if the registry can't be written, never failing it.
#[must_use]
pub fn register(command: &str, repo: &Path, model: &str) -> Option<Registered> {
    let dir = dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("{}.json", std::process::id()));
    let run = ActiveRun {
        pid: std::process::id(),
        command: command.to_owned(),
        repo: repo.display().to_string(),
        model: model.to_owned(),
        started_at: crate::log::now_secs(),
    };
    std::fs::write(&path, serde_json::to_string(&run).ok()?).ok()?;
    Some(Registered(path))
}

/// The runs currently recorded in the registry, for `arc status`, plus any `.json` entries that
/// couldn't be read or parsed (returned, not dropped, so `arc status` surfaces them). A missing
/// registry is not an error — nothing is in flight; a genuine read failure (of the directory or an
/// entry) is, distinguished like [`crate::log::count`] so a permission/IO error can't masquerade as
/// "no active runs". A marker normally clears on exit; a hard-killed process can leave a stale one.
pub fn active() -> anyhow::Result<(Vec<ActiveRun>, Vec<PathBuf>)> {
    let Some(dir) = dir() else {
        return Ok((Vec::new(), Vec::new()));
    };
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), Vec::new())),
        Err(e) => {
            return Err(e).with_context(|| format!("cannot read the run registry {}", dir.display()));
        }
    };
    let mut runs = Vec::new();
    let mut unreadable = Vec::new();
    for entry in entries {
        let path = entry
            .with_context(|| format!("cannot read an entry in the run registry {}", dir.display()))?
            .path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        match std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<ActiveRun>(&text).ok())
        {
            Some(run) => runs.push(run),
            None => unreadable.push(path),
        }
    }
    Ok((runs, unreadable))
}
