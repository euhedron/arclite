use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::cli::{GlobalArgs, InspectArgs};
use crate::output::emit;

/// Fixed manifest filenames that hint at a repo's stack/ecosystem — matched *anywhere*
/// in the tree (real repos nest them in subprojects), gitignore-aware so vendored copies
/// (e.g. `node_modules`) are excluded.
const MANIFEST_NAMES: &[&str] = &[
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "setup.py",
    "requirements.txt",
    "go.mod",
    "pom.xml",
    "build.gradle",
    "Gemfile",
    "composer.json",
    "CMakeLists.txt",
];

/// Manifest file *extensions* (e.g. .NET projects/solutions), reported as `*.ext`.
const MANIFEST_EXTS: &[&str] = &["csproj", "sln", "fsproj", "vbproj"];

/// How many top extensions the human `inspect` view lists before summarizing the rest.
const TOP_EXTENSIONS: usize = 10;

#[derive(Debug, Serialize)]
pub struct InspectReport {
    pub path: String,
    pub is_git_repo: bool,
    pub files: usize,
    pub dirs: usize,
    pub bytes: u64,
    pub manifests: Vec<String>,
    /// Relative paths of the manifest files actually found (root *or* nested) — what the synthesis
    /// context includes, so the scan's findings and that context can't drift apart.
    pub manifest_paths: Vec<String>,
    pub by_extension: BTreeMap<String, usize>,
    /// Entries the walk couldn't read (permission denied, I/O, …) — counted, never silently dropped.
    pub walk_errors: usize,
}

/// Walk a repository/directory and collect structured facts, returning them with the resolved
/// absolute root (so callers reuse it rather than re-resolving).
pub fn gather(path: &Path) -> anyhow::Result<(InspectReport, PathBuf)> {
    anyhow::ensure!(path.exists(), "cannot access {}", path.display());
    let root = super::resolve_root(path)?;

    let mut files = 0usize;
    let mut dirs = 0usize;
    let mut bytes = 0u64;
    let mut by_extension: BTreeMap<String, usize> = BTreeMap::new();
    let mut manifest_types: BTreeSet<String> = BTreeSet::new();
    let mut manifest_paths: Vec<String> = Vec::new();

    // Respect .gitignore, include dotfiles, but never descend into .git internals.
    // Walk errors (permission denied, I/O, …) are counted and reported, not swallowed.
    let (entries, mut walk_errors) = crate::walk::entries(&root);

    for entry in entries {
        if entry.depth() == 0 {
            continue; // the root itself
        }
        let path = entry.path();
        match entry.file_type() {
            Some(ft) if ft.is_dir() => dirs += 1,
            Some(ft) if ft.is_file() => {
                files += 1;
                match entry.metadata() {
                    Ok(md) => bytes += md.len(),
                    // couldn't stat a walked file — surface it via walk_errors, don't drop its size
                    Err(_) => walk_errors += 1,
                }
                let mut is_manifest = false;
                if let Some(name) = path.file_name().and_then(|n| n.to_str())
                    && MANIFEST_NAMES.contains(&name)
                {
                    manifest_types.insert(name.to_owned());
                    is_manifest = true;
                }
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("(none)")
                    .to_owned();
                if MANIFEST_EXTS.contains(&ext.as_str()) {
                    manifest_types.insert(format!("*.{ext}"));
                    is_manifest = true;
                }
                if is_manifest && let Ok(rel) = path.strip_prefix(&root) {
                    manifest_paths.push(rel.to_string_lossy().into_owned());
                }
                *by_extension.entry(ext).or_insert(0) += 1;
            }
            _ => {}
        }
    }

    let manifests: Vec<String> = manifest_types.into_iter().collect();
    manifest_paths.sort();

    let report = InspectReport {
        path: root.display().to_string(),
        is_git_repo: root.join(".git").exists(),
        files,
        dirs,
        bytes,
        manifests,
        manifest_paths,
        by_extension,
        walk_errors,
    };
    Ok((report, root))
}

/// The `inspect` command.
pub fn run(args: &InspectArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    let (report, _root) = gather(&args.path)?;

    let mut ranked: Vec<(&String, &usize)> = report.by_extension.iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    let top = ranked
        .iter()
        .take(TOP_EXTENSIONS)
        .map(|(ext, count)| format!("  {ext:<14} {count}"))
        .collect::<Vec<_>>()
        .join("\n");
    // Surface the elision so the text view doesn't silently hide extensions (--json has them all).
    let top = match report.by_extension.len().saturating_sub(TOP_EXTENSIONS) {
        0 => top,
        more => format!("{top}\n  … +{more} more (--json for all)"),
    };

    let mut human = format!(
        "path       {}\ngit repo   {}\nfiles      {}\ndirs       {}\nbytes      {}\nmanifests  {}\ntop extensions:\n{}",
        report.path,
        report.is_git_repo,
        report.files,
        report.dirs,
        report.bytes,
        if report.manifests.is_empty() {
            "(none)".to_owned()
        } else {
            report.manifests.join(", ")
        },
        if top.is_empty() {
            "  (none)".to_owned()
        } else {
            top
        },
    );
    // Surface the walk's gitignore filtering so the counts above aren't read as the whole tree.
    human.push_str(&format!("\nscope      {}", crate::walk::SCOPE_NOTE));
    // Surface unreadable entries (kept out of the counts above) so the scan isn't quietly partial.
    if report.walk_errors > 0 {
        human.push_str(&format!(
            "\nwalk errors {} (entries arclite couldn't read — see --json)",
            report.walk_errors
        ));
    }

    emit(&report, &human, global.json)
}
