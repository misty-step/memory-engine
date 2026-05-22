---
shaping: true
ticket: 27-ai-content-generation-probe
slice: 6
status: shipped
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

Create a beta generation workflow that turns source material into validated
quiz and exercise drafts with provenance, citations, and rejection reasons
before anything enters the review queue.

Start with deterministic fixtures and adapter-shaped seams. Real provider calls
can come later behind the beta boundary after the generated-content contract is
testable.

## Non-Goals

- No provider SDKs, prompts, network calls, vector stores, or file ingestion in
  `src/`.
- No automatic promotion of AI output into testkit or public package fixtures.
- No free-form tutor chat.
- No generated content without source provenance.
- No assumption that every generated learning item is a quiz. Exercises and
  practice problems are first-class beta content when the domain calls for
  problem-solving rather than definition recall.

## Oracle

- [ ] `experiments/beta-generation/` consumes persisted source material and
      emits generated quiz/exercise draft records with source ids, reference
      spans, model/provider metadata, validation status, and critique notes.
- [ ] Tests cover accepted drafts, rejected unsupported drafts, duplicate-ish
      drafts, and drafts missing required provenance.
- [ ] A saved draft can be promoted into canonical prompt or exercise, queue,
      progression, and schedule inputs consumed by the beta persistence spine.
- [ ] `docs/beta/content-generation.md` documents the ingest -> normalize ->
      generate -> critique -> approve/save workflow and eval gaps.
- [ ] `bun run ci` exits 0.

## Notes

- Embeddings and retrieval are useful for similarity and source selection, not
  proof of truth. Record retrieval/generation receipts separately.
- The first useful product loop is narrow: paste text, generate a few
  quiz/exercise drafts, approve them, review or solve them, and inspect
  failures.
- Philosophical product note: quizzes test whether the learner can retrieve
  something; exercises build deeper understanding by making the learner work
  through a procedure, judgment, or problem. For domains like options trading,
  repeated practice problems about Gamma, hedging, payoff curves, or scenario
  analysis may teach more than repeatedly quizzing the definition of Gamma.

## Closure Evidence

- Implemented in `experiments/beta-generation/` with deterministic,
  source-grounded quiz and exercise draft generation.
- Documented in `docs/beta/content-generation.md`.
- Verified during backlog hygiene with `bun test experiments/beta-store/
  experiments/beta-generation/`.
