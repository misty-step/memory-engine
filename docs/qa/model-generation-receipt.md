# QA receipt — model-backed generation on arbitrary prose

Date: 2026-06-11. Legacy work item: memory-engine-043.

Live walk of the running `memory-engine-api` binary (file store) with
`OPENROUTER_API_KEY` set, default model `google/gemini-3.5-flash`.

## Setup

```sh
PORT=8799 MEMORY_ENGINE_ENABLE_FILE_STORE=true \
  MEMORY_ENGINE_API_STORE_DIR=/tmp/me-qa-store \
  MEMORY_ENGINE_AUTH_ALLOWED_EMAILS=phraznikov@gmail.com \
  MEMORY_ENGINE_AUTH_LINK_OUTBOX_PATH=/tmp/me-qa-store/outbox.jsonl \
  ./target/debug/memory-engine-api
```

## Oracle 1 — arbitrary prose yields ≥1 reviewable draft with provenance

`POST /app/start` with a 3-sentence paragraph about the Antikythera mechanism
(no structured `Concept:/Question:/Answer:` blocks) → HTTP 200, "Generated
material" with **4 drafts**. The persisted store recorded 4 reference spans,
each a verbatim quote of the source:

- "The Antikythera mechanism is an ancient Greek hand-powered device used to
  predict astronomical positions and eclipses…"
- "Discovered in 1901 in a shipwreck off the Greek island of Antikythera"
- "it is considered the oldest known analogue computer."

The generation run recorded `provider: openrouter`, `model:
google/gemini-3.5-flash`, `usage: {inputTokens: 595, outputTokens: 1873,
costUsdMicros: 17750, latencyMs: 2684}` — i.e. **$0.0178 for the source**,
2.7s.

## Oracle 2 — a model-generated draft is fully reviewable

Keep → review → reveal → submit on the first Antikythera draft: approve 200
(renders "Review"/"Reveal answer"), reveal 200 (renders the prompt with its
source reference), submit 200 (renders "Last result", "Next review",
attempts). The model-generated draft flows through grading and scheduling
identically to structured drafts.

## Oracle 3 — per-source cost is logged

Confirmed above: `costUsdMicros` is persisted on the run. (Budget note: the
default model runs ~$0.018–0.025/source, a deliberate quality choice above
the ticket's original $0.02 estimate; see
docs/evals/generation-field-2026-06-11.md.)

## Oracle 4 — structured-block generation still works and is attributed correctly

`POST /app/start` with a structured `Concept:/Question:/Answer:` block → HTTP
200, draft rendered, and the run recorded `model:
deterministic-beta-generator` with `usage: None` (no model call). A bug found
during this QA — the composite `FallbackProvider` mislabeled structured runs
with the fallback model's name — was fixed so each draft is attributed to the
provider that actually produced it.

## Oracle 5 — zero-draft / failure outcomes are visible

`BetaStudyView.generation_notices` carries provider failures, rejected-draft
reasons, and an explicit empty-result sentence, rendered as a "Generation
notes" section. Covered by deterministic tests
(`memory-engine-study::surfaces_a_human_readable_notice_when_a_source_yields_no_drafts`,
`memory-engine-api::generation_notices_render_as_a_visible_section`) rather
than a live model call, since forcing the model to return nothing is not
deterministic.

## Not covered here

Deployment to Fly (gated deploys, abuse limits) is ticket 044. This receipt
is the local-binary dogfood; the same code path serves the deployment.
