---
shaping: true
ticket: 27-ai-content-generation-probe
slice: 6
status: ready
priority: high
estimate: M
depends_on: [26-beta-persistence-spine]
oracles:
  - bun run ci
  - bun test experiments/beta-generation/
  - test -f docs/beta/content-generation.md
---

# AI content generation probe - provenance before promotion

## Goal

Create a beta generation workflow that turns source material into validated quiz
drafts with provenance, citations, and rejection reasons before anything enters
the review queue.

Start with deterministic fixtures and adapter-shaped seams. Real provider calls
can come later behind the beta boundary after the generated-content contract is
testable.

## Non-Goals

- No provider SDKs, prompts, network calls, vector stores, or file ingestion in
  `src/`.
- No automatic promotion of AI output into testkit or public package fixtures.
- No free-form tutor chat.
- No generated content without source provenance.

## Oracle

- [ ] `experiments/beta-generation/` consumes persisted source material and
      emits `GeneratedPromptDraft` records with source ids, reference spans,
      model/provider metadata, validation status, and critique notes.
- [ ] Tests cover accepted drafts, rejected unsupported drafts, duplicate-ish
      drafts, and drafts missing required provenance.
- [ ] A saved draft can be promoted into canonical prompt, queue, progression,
      and schedule inputs consumed by the beta persistence spine.
- [ ] `docs/beta/content-generation.md` documents the ingest -> normalize ->
      generate -> critique -> approve/save workflow and eval gaps.
- [ ] `bun run ci` exits 0.

## Notes

- Embeddings and retrieval are useful for similarity and source selection, not
  proof of truth. Record retrieval/generation receipts separately.
- The first useful product loop is narrow: paste text, generate a few quiz
  drafts, approve them, review them, and inspect failures.
