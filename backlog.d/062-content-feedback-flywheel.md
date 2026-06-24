# In-app content-feedback flywheel

Priority: P2 · Status: pending · Estimate: M

## Goal

Let a learner judge a generated card's *quality* in the moment — 👍/👎 plus an
optional free-text "why" — and capture that verdict tied to the **exact
generation config** that produced the card (prompt variant / model / params), in
an append-only store that **exports to the existing eval harness**. This turns
normal dogfooding into a continuous supply of the human labels 058's `calibrate`
needs and the failure fixtures 060 wants — closing the prompt/context/harness
iteration loop instead of relying on one-off manual labeling sessions.

Homebrew-thin in Rust, not an off-the-shelf platform: the provenance spine
already exists, the grading half is `memory-engine-bench`, and a Python/TS
observability service (Langfuse/Phoenix) would import a mixed-language seam the
invariants resist. Full build-vs-buy analysis:
`docs/research/feedback-flywheel.md`.

## Oracle

- [ ] An append-only `ContentFeedback` record exists: binary verdict
      (kept/dropped) + optional rationale + source (human) + account + timestamp,
      keyed to the `reviewUnit`. Core stays pure; the record lives in
      consumer-owned persistence and is JSON-safe. A revised rating supersedes by
      id (latest-wins), never mutates history.
- [ ] A feedback record **resolves to the exact config** that generated the
      card via `reviewUnit → draft → run` (provider, model, prompt variant) — a
      test asserts a 👎 on a card maps back to the prompt variant + model that
      produced it. Config is resolved by join, never duplicated on the row.
- [ ] A review-surface affordance records 👍/👎 + optional "why" on a generated
      card through the typed service-command boundary, idempotent (no
      double-submit). Binary, not a 1–5 Likert. Natural host: the existing
      post-answer feedback panel ("This item:" section).
- [ ] An exporter emits `feedback → {judge_keep, human_keep, question?}` JSON
      that `bench calibrate` (058) consumes directly, and dropped-flagged content
      as a `bench` generation fixture (060). A round-trip test proves the export
      feeds `calibrate` (κ / TPR-TNR) and a fixture.
- [ ] Config + feedback fields are named after the OTel GenAI conventions
      (`gen_ai.prompt.version`, `gen_ai.evaluation.{score.value, explanation}`)
      so a future OTel/Langfuse observability sink needs no vocabulary migration.
      Do **not** depend on the unmerged `gen_ai.task.feedback.*` proposal.

## Notes

The provenance spine is already in place: `GeneratedPromptModel { provider, name,
version }` (version = prompt variant) per draft + `GenerationRun { provider,
model, usage }`, with `reviewUnit → draft → run` queryable. Today's
`BetaStudyFeedback` is 053's post-answer *pedagogical* feedback (shown **to** the
learner), distinct from this *quality* verdict (the learner's judgment **of** the
card) — don't conflate them.

Off-the-shelf fallback if turnkey dashboards ever outweigh architecture fit:
self-host **Langfuse** (MIT, OTLP+REST) and emit OTel spans + `POST /scores` from
Rust — the OTel-named fields above make that additive, not a rewrite. Promptfoo
is the wrong layer (offline eval; duplicates the bench).

Connects to: 058 (calibration labels), 060/061 (eval fixtures + content-type
work), the graduated-difficulty vision, and the "every dogfooded generation bug
becomes a deterministic eval" practice — this ticket is that practice's durable
input pipe.

## Children

1. `ContentFeedback` entity + append-only persistence (consumer-owned, JSON-safe).
2. Typed service-command to record feedback; idempotent.
3. Review-surface 👍/👎 + "why" affordance (post-answer panel).
4. Config-resolution query (`reviewUnit → draft → run → provider/model/variant`).
5. Exporter → `bench calibrate` labels (058) + `bench` fixtures (060).
6. OTel-conventions field naming (hedge for a future Langfuse/observability sink).
