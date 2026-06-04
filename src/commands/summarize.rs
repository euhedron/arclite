use anyhow::bail;
use serde::Serialize;

use crate::ai;
use crate::cli::{GlobalArgs, SummarizeArgs};
use crate::commands::inspect;
use crate::output::emit;

/// Build the synthesis prompt from deterministic repo facts.
fn build_prompt(facts_json: &str) -> String {
    format!(
        "You are assessing a code repository from deterministic facts gathered by a tool.\n\
         Facts (JSON):\n{facts_json}\n\n\
         In 3-5 sentences, give a concise, useful assessment of this repository: what kind \
         of project it appears to be, its apparent stack, and anything notable or worth a \
         closer look. Base it only on the facts provided."
    )
}

#[derive(Serialize)]
struct SummarizeOutput {
    synthesis: String,
    usage: ai::Usage,
}

#[derive(Serialize)]
struct DryRunOutput<'a> {
    dry_run: bool,
    model: Option<&'a str>,
    allowed_tools: &'a [String],
    estimate: ai::Estimate,
    note: &'static str,
    prompt: &'a str,
}

/// Synthesize a brief assessment of a repository (the `summarize` command).
pub fn run(args: &SummarizeArgs, global: &GlobalArgs) -> anyhow::Result<()> {
    let report = inspect::gather(&args.path)?;
    let facts = serde_json::to_string_pretty(&report)?;
    let prompt = build_prompt(&facts);

    if args.dry_run {
        let estimate = ai::estimate(&prompt);
        let note = "estimate counts the prompt only; a real call also loads the model's base system/tool context, which typically dominates the cost — actual usage is reported after the call runs";
        let tools_desc = if args.allow_tool.is_empty() {
            "none (cheapest — pure synthesis uses no tools)".to_owned()
        } else {
            args.allow_tool.join(", ")
        };
        let human = format!(
            "[dry run — no AI call, $0.00]\nmodel    {}\ntools    {}\nprompt   {} chars (~{} tokens)\nnote     {}\n\n{}",
            args.model.as_deref().unwrap_or("(not set)"),
            tools_desc,
            estimate.chars,
            estimate.approx_tokens,
            note,
            prompt,
        );
        let out = DryRunOutput {
            dry_run: true,
            model: args.model.as_deref(),
            allowed_tools: &args.allow_tool,
            estimate,
            note,
            prompt: &prompt,
        };
        return emit(&out, &human, global.json);
    }

    let Some(model) = args.model.as_deref() else {
        bail!(
            "no model set — pass --model <model> (arclite chooses none for you), or use --dry-run to preview the prompt + estimated cost"
        );
    };

    let synthesis = ai::synthesize(&prompt, model, &args.allow_tool)?;
    let out = SummarizeOutput {
        synthesis: synthesis.text,
        usage: synthesis.usage,
    };
    let u = &out.usage;
    let human = format!(
        "{}\n\n— {} | in {}  cache-write {}  cache-read {}  out {} | ${:.4}",
        out.synthesis,
        u.model,
        u.input_tokens,
        u.cache_creation_input_tokens,
        u.cache_read_input_tokens,
        u.output_tokens,
        u.cost_usd,
    );
    emit(&out, &human, global.json)
}
