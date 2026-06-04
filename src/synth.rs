//! Shared synthesis runner for AI-backed commands (`summarize`, `suggest`, …).
//!
//! Keeps every such command cost-transparent and consistent: `--dry-run` previews
//! the *exact* prompt + a token estimate at zero spend (so sub-par output is
//! traceable to its input), and a real call requires an explicit model, runs with
//! a configurable tool allowlist, and reports the actual cost + cache usage.

use anyhow::bail;
use serde::Serialize;

use crate::ai;
use crate::output::emit;

const DRY_RUN_NOTE: &str = "estimate counts the prompt only; a real call also loads the model's base system/tool context, which typically dominates the cost — actual usage is reported after the call runs";

/// Configuration shared by every synthesis-backed command.
pub struct SynthOptions<'a> {
    /// Model id; `None` means unset (a real call errors — arclite picks none for you).
    pub model: Option<&'a str>,
    /// Claude tools to allow (empty = none = cheapest; a cost lever, never defaulted on).
    pub allowed_tools: &'a [String],
    /// Preview the prompt + estimate without calling the model (zero spend).
    pub dry_run: bool,
    /// Emit machine-readable JSON instead of human text.
    pub json: bool,
}

#[derive(Serialize)]
struct SynthOutput {
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

/// Preview (dry-run) or run a synthesis prompt, then emit the result + its cost.
pub fn run(prompt: &str, opts: &SynthOptions) -> anyhow::Result<()> {
    if opts.dry_run {
        let estimate = ai::estimate(prompt);
        let tools_desc = if opts.allowed_tools.is_empty() {
            "none (cheapest — pure synthesis uses no tools)".to_owned()
        } else {
            opts.allowed_tools.join(", ")
        };
        let human = format!(
            "[dry run — no AI call, $0.00]\nmodel    {}\ntools    {}\nprompt   {} chars (~{} tokens)\nnote     {}\n\n{}",
            opts.model.unwrap_or("(not set)"),
            tools_desc,
            estimate.chars,
            estimate.approx_tokens,
            DRY_RUN_NOTE,
            prompt,
        );
        let out = DryRunOutput {
            dry_run: true,
            model: opts.model,
            allowed_tools: opts.allowed_tools,
            estimate,
            note: DRY_RUN_NOTE,
            prompt,
        };
        return emit(&out, &human, opts.json);
    }

    let Some(model) = opts.model else {
        bail!(
            "no model set — pass --model <model> (arclite chooses none for you), or use --dry-run to preview the prompt + estimated cost"
        );
    };

    let synthesis = ai::synthesize(prompt, model, opts.allowed_tools)?;
    let out = SynthOutput {
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
    emit(&out, &human, opts.json)
}
