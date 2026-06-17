//! The one way arclite walks a repo, shared so `inspect` and `synth` can't drift. The exact scope —
//! what the walk includes and excludes — is stated once in [`SCOPE_NOTE`] (which callers surface) and
//! enforced by [`configured`].

use std::path::Path;

use ignore::WalkBuilder;

/// The one statement of the walk's scope — surfaced by callers so the filtering isn't a silent
/// default: a scan's counts and `--include <dir>` context are this filtered view, not the whole tree.
pub const SCOPE_NOTE: &str = "gitignore-aware (the repo's `.gitignore` and `.git/` excluded), but dotfiles are included and parent/global gitignores are off";

/// A configured, gitignore-aware walk of `dir`.
pub fn configured(dir: &Path) -> ignore::Walk {
    WalkBuilder::new(dir)
        .hidden(false)
        .parents(false)
        .git_global(false)
        // Prune `.git` at the source so the single walk path enforces it — callers don't re-filter.
        .filter_entry(|e| e.file_name() != ".git")
        .build()
}

/// Walk `dir` (gitignore-aware) into its entries plus a count of walk errors (permission
/// denied, I/O, cycles, …). Errors are counted, never silently dropped — so callers can
/// surface a partial scan instead of pretending the missing entries never existed.
pub fn entries(dir: &Path) -> (Vec<ignore::DirEntry>, usize) {
    let mut entries = Vec::new();
    let mut errors = 0usize;
    for result in configured(dir) {
        match result {
            Ok(entry) => entries.push(entry),
            Err(_) => errors += 1,
        }
    }
    (entries, errors)
}
