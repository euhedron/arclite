use std::collections::BTreeMap;

use anyhow::Context;
use ignore::WalkBuilder;
use serde::Serialize;

use crate::cli::{GlobalArgs, InspectArgs};
use crate::output::emit;

/// Manifest files that hint at a repo's stack/ecosystem.
const MANIFESTS: &[&str] = &[
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

#[derive(Serialize)]
struct InspectReport {
    path: String,
    is_git_repo: bool,
    files: usize,
    dirs: usize,
    bytes: u64,
    manifests: Vec<String>,
    by_extension: BTreeMap<String, usize>,
}

/// Walk a repository and report structured facts about it. Deterministic — no LLM.
pub fn run(args: &InspectArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    anyhow::ensure!(args.path.exists(), "cannot access {}", args.path.display());
    let root = std::path::absolute(&args.path)
        .with_context(|| format!("cannot resolve {}", args.path.display()))?;

    let mut files = 0usize;
    let mut dirs = 0usize;
    let mut bytes = 0u64;
    let mut by_extension: BTreeMap<String, usize> = BTreeMap::new();

    // Respect .gitignore, include dotfiles, but never descend into .git internals.
    let walk = WalkBuilder::new(&root)
        .hidden(false)
        .parents(false)
        .git_global(false)
        .build();

    for entry in walk {
        let Ok(entry) = entry else { continue };
        if entry.depth() == 0 {
            continue; // the root itself
        }
        let path = entry.path();
        if path
            .components()
            .any(|c| c.as_os_str().to_str() == Some(".git"))
        {
            continue;
        }
        match entry.file_type() {
            Some(ft) if ft.is_dir() => dirs += 1,
            Some(ft) if ft.is_file() => {
                files += 1;
                if let Ok(md) = entry.metadata() {
                    bytes += md.len();
                }
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("(none)")
                    .to_owned();
                *by_extension.entry(ext).or_insert(0) += 1;
            }
            _ => {}
        }
    }

    let manifests: Vec<String> = MANIFESTS
        .iter()
        .filter(|m| root.join(m).is_file())
        .map(|m| (*m).to_owned())
        .collect();

    let report = InspectReport {
        path: root.display().to_string(),
        is_git_repo: root.join(".git").exists(),
        files,
        dirs,
        bytes,
        manifests,
        by_extension,
    };

    let mut ranked: Vec<(&String, &usize)> = report.by_extension.iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    let top = ranked
        .iter()
        .take(10)
        .map(|(ext, count)| format!("  {ext:<14} {count}"))
        .collect::<Vec<_>>()
        .join("\n");

    let human = format!(
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

    emit(&report, &human, global.json)
}
