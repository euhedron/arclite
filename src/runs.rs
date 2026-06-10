//! Active-run registry: each in-flight synthesis writes its own marker to
//! `~/.arc/runs/<pid>-<index>.json`, updates it as the run streams, and removes it on completion (via
//! the returned guard). One file per run — so a `--runs N` fan-out is N independent files, each
//! written only by the thread that owns it: no shared state, no concurrent writes, no locking. This
//! is the live, ephemeral view; the durable record of every completed run is the log in `log.rs`.

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

/// One in-flight run, as recorded in the registry: static identity plus live, streamed progress.
#[derive(Serialize, Deserialize)]
pub struct ActiveRun {
    pub pid: u32,
    /// Distinguishes the concurrent runs of a `--runs N` fan-out within one process.
    pub index: usize,
    pub command: String,
    pub repo: String,
    pub model: String,
    pub started_at: u64,
    /// Live progress, updated as the run streams. Every field is written at registration, so there
    /// are no serde defaults: a marker that can't be read in full surfaces as unreadable rather
    /// than as fabricated zeros.
    pub turns: u64,
    pub tool_calls: u64,
    /// Characters — not tokens — because the exact token count arrives only at message end (see
    /// [`crate::ai::synthesize`]); characters are the continuous live signal, and the billed token
    /// count lands in the final run report.
    pub output_chars: u64,
}

/// The registry directory, `~/.arc/runs/` (`None` if the home directory is unknown).
fn dir() -> Option<PathBuf> {
    Some(crate::arc_home()?.join("runs"))
}

/// A registered in-flight run: owns its marker and live stats, written only by the single thread that
/// runs it. Removes the marker on drop — success, error, or unwind.
pub struct Active {
    marker: PathBuf,
    run: ActiveRun,
}

impl Active {
    /// Add streamed output characters to the live tally and rewrite the marker (why characters:
    /// see [`ActiveRun::output_chars`]).
    pub fn record_text(&mut self, chars: u64) {
        self.run.output_chars += chars;
        self.write();
    }

    /// Mark one completed turn (and any tool calls it made) and rewrite the marker.
    pub fn record_turn(&mut self, tool_calls: u64) {
        self.run.turns += 1;
        self.run.tool_calls += tool_calls;
        self.write();
    }

    /// Rewrite the marker. `register` already proved the directory writable with the same write, so a
    /// later failure is genuinely exceptional — and it can't be allowed to abort the run it's only
    /// observing, so it's dropped. The durable record is the completed-run log, not this marker.
    fn write(&self) {
        let _ = self.try_write();
    }

    fn try_write(&self) -> std::io::Result<()> {
        let json = serde_json::to_string(&self.run).map_err(std::io::Error::other)?;
        std::fs::write(&self.marker, json)
    }
}

impl Drop for Active {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.marker);
    }
}

/// Record an in-flight run; the returned guard clears its marker on drop. `index` distinguishes the
/// runs of a `--runs N` fan-out within one process. Best-effort: returns `None` (the run proceeds
/// untracked) if the registry can't be written, never failing the run.
#[must_use]
pub fn register(command: &str, repo: &Path, model: &str, index: usize) -> Option<Active> {
    let dir = dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let marker = dir.join(format!("{}-{index}.json", std::process::id()));
    let active = Active {
        marker,
        run: ActiveRun {
            pid: std::process::id(),
            index,
            command: command.to_owned(),
            repo: repo.display().to_string(),
            model: model.to_owned(),
            started_at: crate::log::now_secs(),
            turns: 0,
            tool_calls: 0,
            output_chars: 0,
        },
    };
    active.try_write().ok()?;
    Some(active)
}

/// The runs currently recorded in the registry, for `arc status`, plus any `.json` entries that
/// couldn't be read or parsed (returned, not dropped, so `arc status` surfaces them). A missing
/// registry is not an error — nothing is in flight; a genuine read failure (of the directory or an
/// entry) is — [`crate::read_optional`]'s absent-vs-failed distinction, applied to a directory
/// listing. A marker normally clears on exit; a hard-killed process can leave a stale one.
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
