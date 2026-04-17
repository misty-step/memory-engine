---
shaping: true
ticket: 07-progression-primitives
slice: 2
status: shipped
priority: high
estimate: M
depends_on: []
oracles:
  - bun run ci
  - bun test tests/progression/
---

# Progression primitives — mastery policy, prerequisites, supersession

## Goal

Extract canonical progression metadata and pure eligibility helpers that can
express Vault's `requires` / `supersedes` rules and Ruminatio's stage unlocks
without hardcoding any single app's mastery threshold.

## Non-Goals

- No queue ordering or recent-history anti-clumping. That is ticket 08.
- No session burst planner or UI-facing session plan.
- No package split or storage adapter work.
- No rubric grading.

## Oracle

- [ ] `bun test tests/progression/mastery.test.ts` passes. It proves the
      progression helpers accept an injected mastery policy and reproduces at
      least Vault-style and Ruminatio-style mastery thresholds.
- [ ] `bun test tests/progression/eligibility.test.ts` passes. Cases are lifted
      from Vault queue semantics (`requires`, `supersedes`) and Ruminatio's
      later-stage lock/unlock behavior.
- [ ] `bun run ci` exits 0.

## Notes

- Implementation likely lives in `src/progression.ts` with additive type support
  in `src/types.ts`.
- `Prompt` stays presentation-only. Progression metadata is keyed separately by
  `ReviewUnitId`.
- If a helper cannot operate on shallow JSON-ish inputs and needs consumer ORM
  documents, the API is wrong.
- Study:
  - `/Users/phaedrus/Documents/daybook/tools/vault-srs/src/queue.ts`
  - `/Users/phaedrus/Documents/daybook/tools/vault-srs/tests/queue.test.ts`
  - `/Users/phaedrus/Development/ruminatio/convex/lib/progression.ts`
  - `/Users/phaedrus/Development/ruminatio/apps/web/app/study-system.test.ts`

## What Was Built

- Branch: `feat/07-progression-primitives`
- Result: added canonical progression metadata plus pure mastery and
  eligibility helpers in `src/progression.ts`, exported the new surface from
  the package root, locked the behavior with progression oracle tests, and
  hardened the repo's authoritative Dagger gate with coverage enforcement,
  Gitleaks scanning, and a tracked pre-push hook.
