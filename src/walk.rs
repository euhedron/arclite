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
        .build()
}

/// Whether `path` lies inside a `.git` directory (which arclite never descends into).
pub fn in_git_dir(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str() == ".git")
}
