# Prose→quiz generation: prior art, hypotheses, experiment design

Date: 2026-06-11. Supports tickets 043 (model-backed generation) and 047
(generation eval harness). Model facts rot in weeks; re-verify pricing before
re-running the bench.

## Prior art

**Scry (`../scry`, production prompts in `convex/lib/promptTemplates.ts`).**
Topic→quiz, three-stage pipeline (intent extraction → concept synthesis →
phrasing generation), Gemini 3 Pro with thinking. Hard-won prompt lessons,
adopted here:

- Reasoning-tier models do best with simple direct task descriptions,
  principles over procedures, **no** chain-of-thought scaffolding, **no**
  few-shot examples.
- One concept per atomic unit; concept titles ≤ 12 words; no vague labels.
- Questions must be standalone (never depend on surrounding context).
- MCQ distractors must be semantically adjacent confusions, never
  punctuation/format variants; 3–4 options.
- Generate rationale first to focus the item, return only the final payload.

Key difference: Scry's input is a *topic* (model free to use world
knowledge); memory-engine's input is a *source document* and every draft must
carry provenance quoting that source. Ours is grounded question generation,
which changes the quality bar from "plausible" to "verifiable".

**Grounding literature (verified 2026-06).** The field converged on exactly
the trust-gate shape this repo already has:

- Citation/evidence enforcement cuts fabrication from ~8–15% to under 3%
  (production RAG reports, neelmishra.github.io citation-grounding).
- "Evidence Grounding Score" pattern (VERDI, arXiv 2605.11334): extract the
  model's quoted spans, fuzzy-match each against the source with token-level
  overlap (~80% threshold); fabricated quotes → low score. Deterministic,
  no model in the judge loop.
- Iterative generate-judge-repair loops (De Jure, arXiv 2604.02276) raise
  quality but multiply latency/cost; deterministic post-hoc validation plus
  rejection is the cheap alternative and matches our existing
  `GeneratedPromptValidation` gate.
- Open-source flashcard generators (anki-llm, RemNote generator, AI-Question-
  Generation) use single-pass generation with schema-validated output and
  semantic chunking for long inputs; none do multi-stage pipelines.

## Model field (pricing verified 2026-06-11, USD per 1M tokens)

| Model (OpenRouter id) | In | Out | Notes |
| --- | --- | --- | --- |
| `deepseek/deepseek-v4-flash` | $0.112 | $0.224 | 1M ctx, structured outputs, highest-throughput OR model (2T tok/wk) |
| `openai/gpt-5.4-nano` | $0.20 | $1.25 | small-model tier, strong instruction following |
| `qwen/qwen3.6-flash` | $0.1875 | $1.125 | 25%-off promo, JSON mode + structured output |
| `google/gemini-3.1-flash-lite` | ~$0.13–0.25 | ~$1.50 | fastest/cheapest Gemini; weaker reasoning |
| `google/gemini-3-flash` | $0.50 | $3.00 | quality anchor: wins FACTS Grounding vs Flash-Lite |
| `anthropic/claude-haiku-4.5` | $1.00 | $5.00 | premium small model; likely over budget at scale |

All support JSON-schema-constrained output through OpenRouter
`response_format: {type: "json_schema", strict: true}` with
`require_parameters: true` provider routing.

## Hypotheses

- **H1 (pipeline).** Single-pass, JSON-schema-constrained generation with a
  required verbatim `evidence_quote` per draft, gated by deterministic
  post-hoc validation (quote-in-source, duplicate suppression), reaches
  ≥80% provenance-verified drafts on flash-tier models. No multi-stage
  pipeline or critique loop needed at beta scale.
- **H2 (model).** DeepSeek V4-flash is within a few points of Gemini 3 Flash
  on the deterministic judges at roughly a tenth of the cost, making it the
  default; Gemini 3 Flash is the quality fallback.
- **H3 (faithfulness mechanics).** Cheap models paraphrase rather than quote;
  exact substring matching will under-credit. Judges need normalized
  (case/whitespace/punctuation-folded) matching, and the provenance gate
  should use the same normalization to avoid rejecting good drafts.
- **H4 (latency).** Single-pass generation on a ~3k-token source completes
  in < 10s p50 on flash-tier models — acceptable for the paste→generate UX.
  A Scry-style multi-stage pipeline would multiply that 2–3× for unproven
  quality gain on grounded input.
- **H5 (cost).** A typical source (~3k tokens in, ~1.5k out) costs
  $0.0007–$0.0050 on the shortlist — comfortably under the ticket's $0.02
  budget, so quality and latency, not cost, should drive the model pick.

## Experiment design (first pass)

Corpus: ≥10 real-world texts (prose, lists, technical doc, narrative; varied
length) with expected-property annotations, committed in-repo.

Configurations: shortlist models × 2 prompt variants
(A: minimal direct instruction; B: principle-rich, Scry-informed rules).

Deterministic judges, no model in the loop:

1. Schema-validity rate (parse failures / responses).
2. Provenance rate: `evidence_quote` found in source after normalization.
3. Answerability: answer content findable in source (token-overlap).
4. Duplicate rate across drafts of one source.
5. Draft-count distribution vs annotated expectations.

Measures per run: judge scores, prompt/completion tokens, dollars (OpenRouter
usage accounting), wall-clock latency p50/p95.

Receipts land in `docs/evals/` with dates; the default model is named from
receipts, not vibes (047 oracle).
