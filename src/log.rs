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

/// The arclite logs directory, `~/.arc/logs` — the single source the run log and the result store
/// both build on (`None` only if the home directory can't be determined).
fn logs_dir() -> Option<PathBuf> {
    Some(crate::arc_home()?.join("logs"))
}

/// Path of the run log, `~/.arc/logs/runs.jsonl`. Both [`append`] and `doctor` use it.
pub fn path() -> Option<PathBuf> {
    Some(logs_dir()?.join("runs.jsonl"))
}

/// The file storing one run's full result, `~/.arc/logs/results/<id>.json` — read by `arc log <id>`.
pub fn result_path(id: &str) -> Option<PathBuf> {
    Some(logs_dir()?.join("results").join(format!("{id}.json")))
}

/// Number of run records currently logged — for `doctor`. `Ok(0)` when the log is absent (no runs
/// yet), `Ok(n)` for a readable log, and `Err` when it exists but can't be read: an unreadable log
/// is surfaced distinctly rather than silently shown as 0, which would hide a dropped/corrupt log.
pub fn count() -> std::io::Result<usize> {
    let Some(p) = path() else { return Ok(0) };
    match std::fs::read_to_string(&p) {
        Ok(text) => Ok(text.lines().filter(|l| !l.trim().is_empty()).count()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(e),
    }
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

/// Store one run's full result at [`result_path`] (best-effort: a failure warns but never fails the
/// run, like [`append`]). Returns the path written, or `None` if it couldn't be stored.
pub fn store_result<T: Serialize>(id: &str, content: &T) -> Option<PathBuf> {
    let path = result_path(id)?;
    let write = || -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, serde_json::to_string_pretty(content)?)?;
        Ok(())
    };
    match write() {
        Ok(()) => Some(path),
        Err(e) => {
            eprintln!("arclite: run result not stored ({}): {e}", path.display());
            None
        }
    }
}
