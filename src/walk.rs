//! The one way arclite walks a repo, shared so `inspect` and `synth` can't drift. The exact scope —
//! what the walk includes and excludes — is stated once in [`SCOPE_NOTE`] (which callers surface) and
//! enforced by [`configured`].

use std::path::Path;

use ignore::WalkBuilder;

/// The one statement of the walk's scope — surfaced by callers so the filtering isn't a silent
/// default: a scan's counts and `--include <dir>` context are this filtered view, not the whole tree.
pub const SCOPE_NOTE: &str = "gitignore-aware (the repo's `.gitignore` and `.git/` excluded), but dotfiles are included and parent/global gitignores are off";

/// A configured, gitignore-aware walk of `dir` within `repo_root`. A walk rooted *below* the repo
/// root (an `--include` subdirectory) would otherwise leave the root's `.gitignore` inert — the
/// ignore crate reads ignore files only from the walk root down, and parent lookup is deliberately
/// off ([`SCOPE_NOTE`]) — so the `.gitignore` chain from the repo root to `dir` is seeded
/// explicitly, keeping the note's "the repo's `.gitignore` excluded" claim true for subdir walks
/// (first broken in the wild by a gitignored `__pycache__` riding an `--include` of a Python
/// package). When `dir` *is* the repo root the chain is empty and this is the plain rooted walk.
pub fn configured(repo_root: &Path, dir: &Path) -> ignore::Walk {
    let mut builder = WalkBuilder::new(dir);
    builder
        .hidden(false)
        .parents(false)
        .git_global(false)
        // Prune `.git` at the source so the single walk path enforces it — callers don't re-filter.
        .filter_entry(|e| e.file_name() != ".git");
    if dir != repo_root && dir.starts_with(repo_root) {
        for ancestor in dir.ancestors().skip(1) {
            let gitignore = ancestor.join(".gitignore");
            if gitignore.is_file() {
                // A malformed or unreadable chain file degrades to a less-filtered walk —
                // over-inclusion (visible in the disclosed context sizes), never data loss —
                // matching the walker's own leniency for the ignore files it reads in-walk.
                builder.add_ignore(gitignore);
            }
            if ancestor == repo_root {
                break;
            }
        }
    }
    builder.build()
}

/// Walk `dir` within `repo_root` (gitignore-aware — see [`configured`]) into its entries plus a
/// count of walk errors (permission denied, I/O, cycles, …). Errors are counted, never silently
/// dropped — so callers can surface a partial scan instead of pretending the missing entries never
/// existed.
pub fn entries(repo_root: &Path, dir: &Path) -> (Vec<ignore::DirEntry>, usize) {
    let mut entries = Vec::new();
    let mut errors = 0usize;
    for result in configured(repo_root, dir) {
        match result {
            Ok(entry) => entries.push(entry),
            Err(_) => errors += 1,
        }
    }
    (entries, errors)
}
