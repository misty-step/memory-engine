---
shaping: true
ticket: 32-graduated-activity-ladder
slice: 6
status: ready
priority: medium
estimate: L
depends_on: [28-mobile-beta-study-interface, 29-service-contract-v0-hardening]
oracles:
  - bun run ci
  - cargo test -p memory-engine-study -p memory-engine-beta-app
  - test -f docs/beta/graduated-activity-ladder.md
---

# Graduated activity ladder - from quiz recall to exercises

## Goal

Add a beta-owned activity ladder that lets one concept progress through
multiple activity forms: recognition, cued recall, free recall, composition,
and practice exercises.

This is the path away from memorizing a card and toward understanding the
underlying concept. The beta should start with deterministic variants and
explicit metadata before attempting broad AI-generated exercises.

## Non-Goals

- No public package export in this ticket.
- No provider SDKs, prompt templates, vector stores, or generation network calls
  under `crates/memory-engine-core`.
- No assumption that FSRS alone decides pedagogical stage transitions.
- No unlimited exercise generation before deterministic templates and QA
  receipts prove the shape.
- No broad domain authoring system; start with one or two narrow fixtures.

## Oracle

- [ ] The Rust beta study app represents at least one concept with multiple
      activity variants and a shared concept/progression group.
- [ ] Tests prove mastery of simpler variants can unlock harder variants
      without duplicating schedule history or losing attempt provenance.
- [ ] Multiple-choice variants shuffle answer choices and can increase choice
      count without changing the underlying concept identity.
- [ ] A typed-recall variant and a composition/exercise variant are both driven
      through the beta service boundary.
- [ ] At least one fixture demonstrates a real ladder, such as NATO phonetic
      alphabet: 3-choice recognition -> 5-choice recognition -> typed recall
      -> spell a word using the alphabet.
- [ ] At least one exercise fixture records a worked solution or scoring rubric
      so QA can distinguish "harder" from merely "different."
- [ ] `docs/beta/graduated-activity-ladder.md` documents the ladder model,
      kernel/application boundary, dogfood friction, and what would justify
      promotion.
- [ ] `bun run ci` exits 0.

## Notes

- The likely kernel-owned pieces are existing progression metadata, queue
  metadata, grading envelopes, and schedule state. The beta app owns wording,
  distractors, activity-kind selection, worked solutions, and generated
  scenarios until repeated evidence proves otherwise.
- Difficulty is not just more answer choices. It can mean less cueing, more
  composition, more confusable distractors, delayed feedback, interleaving, or
  a generated scenario that requires choosing the right strategy.
- For options learning, the long-term target is practice problems that require
  applying concepts such as Gamma to scenarios, not just recalling definitions.
