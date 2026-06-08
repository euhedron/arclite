//! Append-only per-run logging to `~/.arc/logs/runs.jsonl` — an observable trace of every AI run
//! (params, context, tokens, cost): the substrate for "is the spend earning its keep" metrics, and
//! for tracing what actually happened. On by default; disable via `defaults.logging = false` in
//! settings. A write failure is surfaced as a warning but never fails the command.

use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

/// Current UNIX time in seconds.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before the UNIX epoch")
        .as_secs()
}

/// Path of the run log, `~/.arc/logs/runs.jsonl` — the single source for where runs are recorded
/// (`None` only if the home directory can't be determined). Both [`append`] and `doctor` use it.
pub fn path() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(crate::ARC_DIR).join("logs").join("runs.jsonl"))
}

/// Number of run records currently logged (0 if the log is absent or unreadable) — for `doctor`.
pub fn count() -> usize {
    path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map_or(0, |text| text.lines().filter(|l| !l.trim().is_empty()).count())
}

/// Append `record` as one JSON line to the [`path`] run log, returning the path written.
/// Any failure is surfaced as a warning and returns `None` — logging never breaks the command.
pub fn append<T: Serialize>(record: &T) -> Option<PathBuf> {
    let Some(target) = path() else {
        eprintln!("arclite: run not logged (cannot determine the home directory)");
        return None;
    };
    let dir = target.parent().expect("the log path always has a parent").to_path_buf();
    let line = match serde_json::to_string(record) {
        Ok(line) => line,
        Err(e) => {
            eprintln!("arclite: run not logged (could not serialize record): {e}");
            return None;
        }
    };
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(&dir)?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&target)?;
        writeln!(file, "{line}")
    };
    match write() {
        Ok(()) => Some(target),
        Err(e) => {
            eprintln!("arclite: run not logged ({}): {e}", target.display());
            None
        }
    }
}
