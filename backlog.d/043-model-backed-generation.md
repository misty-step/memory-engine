# Make one-input generation work on arbitrary prose

Priority: P0 · Status: pending · Estimate: L

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

## Notes

Live evidence: POST /app/source with plain prose → /app/generate → 200 with
0 drafts and no message. `memory-engine-generation/src/lib.rs:324-340` parses
only structured blocks; `validation_failures` are never rendered
(`memory-engine-api/src/lib.rs` `render_drafts`). There is no model client
anywhere in the workspace. SPEC.md already reserves the provider-adapter
direction. Draft validation (provenance, duplicate suppression) already
exists — reuse it as the trust gate on model output.

## Children

1. Generation provider trait + deterministic fake provider; current parser
   becomes the structured provider.
2. Surface zero-draft/validation outcomes in the study UI (empty state +
   reasons).
3. LLM provider crate (config via env secret, timeouts, output schema
   validation, provenance required) behind the trait.
4. Deployed dogfood receipt: arbitrary prose → kept draft → reviewed on
   phone.
