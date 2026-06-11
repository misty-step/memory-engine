# Make one-input generation work on arbitrary prose

Priority: P0 · Status: ready · Estimate: L

## Goal

A learner pastes any prose and gets a small, trustworthy, source-cited set of
draft review items — the shipped promise of ticket 37, which today only a
hand-written `Concept:/Question:/Answer:` block can satisfy.

## Oracle

- [ ] Pasting unstructured prose (e.g. a paragraph about mitochondria)
      produces ≥1 reviewable draft with provenance quoting the source.
- [ ] Generation failures and zero-draft outcomes render a visible,
      human-readable explanation in the study UI (no silent empty section).
- [ ] Model provider lives behind a boundary crate/trait; core and study
      crates stay provider-free; CI runs with a deterministic fake provider,
      no live model calls.
- [ ] Existing structured-block generation keeps working (it becomes one
      provider among several).
- [ ] Per-source generation cost is logged; a typical source costs < $0.02.

## Notes

Live evidence: POST /app/source with plain prose → /app/generate → 200 with
0 drafts and no message. `memory-engine-generation/src/lib.rs:324-340` parses
only structured blocks; `validation_failures` are never rendered. There is no
model client anywhere in the workspace. Draft validation (provenance,
duplicate suppression) already exists — reuse it as the trust gate on model
output.

**Model research (2026-06-10).** Cheap structured-output models, $/M
input/output, all support JSON-constrained output:

| Model | In | Out | Source |
| --- | --- | --- | --- |
| DeepSeek V4-flash | $0.098 | $0.197 | OpenRouter, roster index 2026-06-07 |
| GPT-5.4-nano | $0.20 | $1.25 | developers.openai.com/api/docs/pricing |
| Gemini 3.1 Flash-Lite | $0.25 | $1.50 | metacto.com Gemini pricing 2026-05-31 |
| Qwen3.6-flash | $0.19 | $1.13 | OpenRouter, roster index 2026-06-07 |
| Gemini 3 Flash | $0.50 | $3.00 | metacto.com / inworld.ai |
| Claude Haiku 4.5 | $1.00 | $5.00 | anthropic.com (Oct 2025, unchanged) |

A ~3k-token source generating ~1.5k tokens of drafts costs $0.0006–$0.01.
Recommendation: speak the OpenAI-compatible chat/completions dialect against
OpenRouter so one provider implementation covers every candidate; pick the
default model from ticket 047's eval results, not vibes. Scry used Gemini
for the same job — its prompts are prior art (`../scry`).

## Children

1. Generation provider trait + deterministic fake provider; current parser
   becomes the structured provider.
2. Surface zero-draft/validation outcomes in the study UI (empty state +
   reasons).
3. OpenRouter-dialect HTTP provider crate: env-secret config, timeouts,
   JSON-schema-constrained output, provenance required, cost logging.
4. Default-model selection driven by 047 eval receipts.
5. Deployed dogfood receipt: arbitrary prose → kept draft → reviewed on
   phone.
