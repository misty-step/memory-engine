---
shaping: true
ticket: 28-mobile-beta-study-interface
slice: 6
status: ready
priority: high
estimate: L
depends_on: [26-beta-persistence-spine, 27-ai-content-generation-probe]
oracles:
  - bun run ci
  - bun test experiments/beta-study/
  - test -f docs/beta/mobile-study.md
---

# Mobile beta study interface - usable local review loop

## Goal

Build a repo-local, mobile-first beta interface that can be used for real
dogfood sessions: add source material, approve generated quiz and exercise
drafts, review items, solve practice problems, persist results, and inspect
what happened.

This is the first interface that should feel useful rather than merely proving
API ergonomics. It may own local persistence and UI state because it lives under
`experiments/`, not the published kernel.

## Non-Goals

- No production deployment, auth, billing, telemetry, or app-store packaging.
- No public service export.
- No database or UI dependency in `src/`.
- No broad content ingestion matrix before pasted text works end to end.
- No extraction until a later decision ticket.
- No quiz-only interaction model. The beta interface should support exercise
  solving where the learning goal is procedural fluency or deep understanding,
  not just factual recall.

## Oracle

- [ ] `experiments/beta-study/` provides a local mobile-responsive interface
      over the beta store and generation probe.
- [ ] A code-level test drives source creation, quiz/exercise draft approval,
      queue selection, answer submission or worked-solution entry, reveal,
      grade/apply-review, next item, and persisted session summary.
- [ ] A restart/resume test proves a learner can quit after at least one saved
      review and resume from persisted state without regenerating content.
- [ ] A duplicate-submit or retry test proves the interface does not corrupt
      schedule history or double-count attempts.
- [ ] A browser smoke receipt verifies the local interface at a phone-sized
      viewport with no horizontal overflow.
- [ ] The interface displays review-state projection, expected answer or worked
      solution after reveal/submit, attempt outcome, schedule change, and next
      queue item.
- [ ] `docs/beta/mobile-study.md` records UX friction, API pressure, and
      whether the database/service boundary still feels right.
- [ ] `bun run ci` exits 0.

## Notes

- Optimize the first screen for action: source input, pending drafts, or next
  review depending on state. Avoid a marketing page.
- The beta must persist enough state that repeated manual sessions can reveal
  real workflow friction.
- The mobile path is the product proof. Desktop can exist, but the dogfood
  oracle should include a phone-sized viewport.
- Beta usefulness requires at least one actual repeated-use dogfood receipt
  after executable fixtures pass.
- Treat exercises as first-class interface work. A learner studying options may
  need practice problems that drill scenario analysis and calculation, not only
  quizzes that check definitions.
