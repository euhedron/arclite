use std::path::Path;

use crate::cli::{GlobalArgs, SynthArgs};
use crate::commands::inspect;
use crate::synth::{self, SynthOptions};

/// Read a file as text, capped at `max_chars` (char-safe) so context stays bounded.
fn read_capped(path: &Path, max_chars: usize) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    if text.chars().count() > max_chars {
        let head: String = text.chars().take(max_chars).collect();
        Some(format!("{head}\n…[truncated]"))
    } else {
        Some(text)
    }
}

/// Assemble the prompt from deterministic facts plus bounded README + manifest text.
fn build_prompt(facts_json: &str, readme: Option<&str>, manifests: &[(String, String)]) -> String {
    let mut context = format!("Deterministic facts (JSON):\n{facts_json}\n");
    if let Some(readme) = readme {
        context.push_str(&format!("\nREADME:\n{readme}\n"));
    }
    for (name, body) in manifests {
        context.push_str(&format!("\n{name}:\n{body}\n"));
    }
    format!(
        "You are reviewing a code repository to advise where attention is best spent.\n\n\
         {context}\n\
         Produce a PRIORITIZED list (most important first) of concrete, useful suggestions — \
         what to look at, verify, improve, finish, or be aware of in this repo — each a short \
         line with a one-clause rationale. Ground every item in the context above; skip anything \
         you can't support from it. Lead with what matters most."
    )
}

/// Synthesize a prioritized list of suggestions for a repository (the `suggest` command).
pub fn run(args: &SynthArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    let report = inspect::gather(&args.path)?;
    let facts = serde_json::to_string_pretty(&report)?;

    let root = std::path::absolute(&args.path).unwrap_or_else(|_| args.path.clone());
    let readme = read_capped(&root.join("README.md"), 4000);
    let manifests: Vec<(String, String)> = report
        .manifests
        .iter()
        .filter_map(|name| read_capped(&root.join(name), 2000).map(|body| (name.clone(), body)))
        .collect();

    let prompt = build_prompt(&facts, readme.as_deref(), &manifests);

    synth::run(
        &prompt,
        &SynthOptions {
            model: args.model.as_deref(),
            allowed_tools: &args.allow_tool,
            dry_run: args.dry_run,
            json: global.json,
        },
    )
}
