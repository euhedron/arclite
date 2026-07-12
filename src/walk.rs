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
pub fn configured(repo_root: &Path, dir: &Path) -> anyhow::Result<ignore::Walk> {
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
            // Absent is the normal case (skip); present joins the chain; unreadable must not
            // collapse into absent (the `optional`/`try_is_dir` standard) — a permission-denied
            // root `.gitignore` silently yielding an unfiltered walk would be a silent scope
            // change, so it surfaces instead. A *malformed* pattern line inside a readable file
            // stays inert (git's own tolerance, matching the walker's in-walk leniency).
            match std::fs::metadata(&gitignore) {
                Ok(m) if m.is_file() => {
                    builder.add_ignore(gitignore);
                }
                Ok(_) => {} // a directory named `.gitignore` is not an ignore file
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(anyhow::anyhow!(
                        "cannot read {} (needed for the walk's ignore chain): {e}",
                        gitignore.display()
                    ));
                }
            }
            if ancestor == repo_root {
                break;
            }
        }
    }
    Ok(builder.build())
}

/// Walk `dir` within `repo_root` (gitignore-aware — see [`configured`]) into its entries plus a
/// count of walk errors (permission denied, I/O, cycles, …). Errors are counted, never silently
/// dropped — so callers can surface a partial scan instead of pretending the missing entries never
/// existed.
pub fn entries(repo_root: &Path, dir: &Path) -> anyhow::Result<(Vec<ignore::DirEntry>, usize)> {
    let mut entries = Vec::new();
    let mut errors = 0usize;
    for result in configured(repo_root, dir)? {
        match result {
            Ok(entry) => entries.push(entry),
            Err(_) => errors += 1,
        }
    }
    Ok((entries, errors))
}
