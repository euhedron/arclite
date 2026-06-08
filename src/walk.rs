//! The one way arclite walks a repo — gitignore-aware, dotfiles included, parent/global
//! gitignores off, never descending into `.git`. Shared so `inspect` and `synth` can't drift.

use std::path::Path;

use ignore::WalkBuilder;

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
