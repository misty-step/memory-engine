# Content-feedback flywheel: off-the-shelf vs homebrew

Research (2026-06-24) on adding a UI layer where a learner rates model-generated
content (👍/👎 + free-text "why"), captured and tied to the **exact generation
config** that produced it (prompt variant / model / params), building a
quantitative + qualitative feedback DB that feeds prompt / context / harness
iteration.

## TL;DR

**Build it homebrew-thin in Rust.** The off-the-shelf tools that do this
(Langfuse is the real one; Promptfoo is the wrong layer) are all Python/TS
services reachable from Rust only over OTLP+REST — net-new mixed-language
surface our invariants resist. And the hard part is already done: every
generated card already records its config, so the capture is one append-only
table keyed to a record we already write, and the grading half is
`memory-engine-bench` + 058's `calibrate`. Langfuse stays the clean fallback if
turnkey dashboards ever outweigh architecture fit; the OTel-named fields keep
that door open.

## Off-the-shelf landscape

| Tool | Thumbs + free-text API | Prompt-version ↔ generation link | OSS / self-host | Rust via REST/OTLP |
|---|---|---|---|---|
| **Langfuse** | ✅ **Scores** — one object = value (`NUMERIC/CATEGORICAL/BOOLEAN/TEXT`) **+ comment** | ✅ first-class; per-version score aggregation | ✅ **MIT**, fully self-host; scores/prompt-mgmt/datasets/judge un-gated | ✅ public REST + **native OTLP/HTTP** (no Rust SDK; community crate exists) |
| **Phoenix (Arize)** | ✅ **span annotations** — `label`+`score`+**`explanation`**, curl-native | ⚠️ convention-driven (OpenInference span attrs you set) | ✅ Elastic-License 2.0 (not OSI), no feature gates in OSS | ✅ cleanest OTLP (gRPC 4317 / HTTP 6006) |
| **Braintrust** | ✅ `logFeedback` — `scores`+`comment`+`expected` | ✅ versioned prompts auto-link to spans | ❌ proprietary; "self-host" = hybrid BYO-cloud + license-gated control plane | ✅ REST + OTLP (Rust SDK alpha) |
| **Helicone** | ⚠️ split: Scores API (int/bool) + Custom Property for text | ✅ `Helicone-Prompt-Id` + auto-version | ⚠️ Apache-2.0 but **maintenance mode** (Mintlify-acquired 2026-03-03) | ❌ **no native OTLP** (OpenLLMetry only) |
| **Promptfoo** | ❌ offline eval only — no in-app capture | ✅ | ✅ MIT (OpenAI-acquired 2026-03-09) | n/a (CLI/CI) |
| *LangSmith* | ✅ `feedback` (score+comment) | ✅ | ❌ proprietary; self-host enterprise-only | ✅ REST + OTLP |

**Reading:** Langfuse is the strongest single off-the-shelf fit and genuinely
does what we want. Phoenix is the OTLP-purist OSS alternative (weaker
prompt-link). Promptfoo is the *wrong layer* — an offline harness that
duplicates `memory-engine-bench` — not a feedback capture tool. Braintrust /
LangSmith fail the OSS / consumer-owned-persistence constraint. Helicone is the
easiest on-ramp (base-URL proxy) but acquired-and-coasting with no native OTLP.

## Why homebrew for Scry

1. **The provenance spine already exists.** Every draft carries
   `GeneratedPromptModel { provider, name, version }` (`version` = the prompt
   variant label) and links to a `GenerationRun { provider, model, usage }`;
   `reviewUnit → draft → run` is queryable. The full config axis is
   `OpenRouterConfig { model, prompt: PromptVariant, max_drafts }`. The universal
   data-model finding across all five tools is *"config lives on the generation
   record; feedback references it by id, resolved by join — never duplicated."*
   We are 90% of the way there.
2. **Buying imports a language seam.** Langfuse/Phoenix are Python/TS services
   reachable only over OTLP+REST. That violates the repo grain (Rust-by-default,
   no non-Dagger TS runtime, consumer-owned persistence) for what is, for us, a
   single append-only insert.
3. **We already own the grading half.** `memory-engine-bench` + 058 `calibrate`
   *is* the LLM-as-judge calibration these platforms sell. A learner's 👍/👎 **is**
   058's `human_keep` (its label file is a JSON array of
   `{judge_keep, human_keep, question?}`). Dropped-flagged content becomes a 060
   fixture. The loop closes inside our existing harness — no external eval engine.
4. **It matches the documented consensus workflow** (below) rather than tooling.

## Consensus workflow (what the feedback feeds)

Hamel Husain's eval posts are the load-bearing methodology; Shankar supplies the
academic backing (criteria drift: judge criteria *can't* be fixed up front, so
calibration must be iterative):

1. Instrument in-product 👍/👎 + free-text, attached to the generation.
2. Make looking at the data frictionless; **sample negatives first**.
3. **Error analysis** — open-code failures into notes, then **axial-code into a
   counted failure taxonomy** (stop at saturation).
4. A single expert makes **binary pass/fail** calls + a written critique.
5. **Promote corrected examples into a versioned golden dataset** (the
   `correction`/`expected` field = the ideal output).
6. **Calibrate an LLM-judge** to the human labels — **TPR/TNR separately** (not
   raw accuracy), **Cohen's κ**, always on a **held-out** split.
7. Iterate prompts/models against the dataset; new production failures feed back
   as regression fixtures.

This is exactly our eval thread — 058 (`calibrate`, κ, TPR/TNR) + 060 (fixtures)
+ the "every dogfooded generation bug becomes a deterministic eval" practice —
made *continuous* by a durable feedback table.

## Homebrew design

- **Entity (append-only):** `ContentFeedback { id, review_unit_id, verdict:
  kept|dropped, rationale: Option<String>, source: human, account_id,
  created_at, metadata }`. Config is resolved by join through
  `reviewUnit → draft → run`, never duplicated on the row. Supersede-by-id
  (latest-wins) if a learner revises. **Binary, not 1–5 Likert** (Hamel: binary
  beats Likert for product feedback). Consumer-owned persistence; core stays pure.
- **UI:** 👍/👎 + optional "why" on each generated card. Natural host is the
  existing post-answer feedback panel (already renders a "This item:" section);
  recorded through the typed service-command boundary.
- **Exporter:** `feedback → {judge_keep, human_keep}` JSON for `bench calibrate`
  (058); dropped-flagged content → a `bench` fixture (060). This *is* the flywheel.
- **OTel hedge (free):** name the config + feedback fields after the OTel GenAI
  semantic conventions (`gen_ai.request.model`, `gen_ai.prompt.version`,
  `gen_ai.evaluation.{score.value, explanation}`) so we can *also* emit OTel
  spans to a self-hosted Langfuse later without a vocabulary migration. Do **not**
  depend on the still-unmerged `gen_ai.task.feedback.*` proposal.

Build vs buy is therefore not either/or: the thin Rust table is the system of
record and feeds the bench; Langfuse is an optional observability sink on top.

## Sources

- Langfuse: scores data model (`langfuse.com/docs/evaluation/scores/data-model`);
  "Doubling Down on Open Source" (2025-06-04). Phoenix annotations
  (`phoenix.arize.com/.../annotations`). Helicone in maintenance mode under
  Mintlify (2026-03-03).
- Promptfoo: docs (`promptfoo.dev/docs/guides/llm-as-a-judge`); OpenAI
  acquisition (`openai.com/index/openai-to-acquire-promptfoo/`, 2026-03-09).
- Workflow: Hamel Husain — evals (2024-03-29), llm-judge (2024-10-29), evals-faq
  (2026-01-15); Shankar — *Who Validates the Validators* (arXiv 2404.12272,
  2024-04-18), AI-engineering flywheel (2024-07-01); Braintrust golden datasets
  (2026-05-21).
- Standards: OpenTelemetry GenAI semantic conventions
  (`github.com/open-telemetry/semantic-conventions`); `gen_ai.task.feedback.*` is
  an unmerged proposal (issue #2664).

## Residual / caveats

- Most vendor doc pages are undated; confirmed current via versioned/dated
  signals. The methodology sources (Hamel ×3, Shankar, Braintrust, OpenAI/arXiv)
  carry firm dates.
- No Rust-native OSS exists for this niche; the closest Rust artifact is
  Helicone's AI Gateway (a routing proxy, not feedback capture).
- Phoenix prompt-version attribution and exact OSS feature parity (Helicone
  docker-compose) are the softest claims — verify hands-on before relying.
