---
shaping: true
slice: 2
parent: SPEC.md
date: 2026-04-17
status: ready-for-implement
---

# Context Packet: Memory Engine — Slice 2 (Progression + Queue + Recitation)

## Goal

Add shared progression metadata, progression-aware queue primitives, and the
remaining deterministic `recitation` prompt to the kernel, while proving the
boundary against a second consumer without forcing a package split.

## Non-Goals

- No rubric or AI-assisted grading. That is slice 3.
- No storage adapters. Consumer persistence stays consumer-owned.
- No monorepo/package split. Keep the current single package and use subpath
  exports only if a new surface needs them.
- No session choreography. Ruminatio's `selectBurst()` and Caesar's session
  builder remain app-owned unless a later canary proves they are truly shared.
- No content parser or authoring pipeline work.

## Constraints / Invariants

- **Core stays pure.** No framework, storage, network, or SDK imports in `src/`.
- **`ReviewUnitId` stays opaque.** Concept-level and phrasing-level consumers
  both map into the same core identifiers; the kernel never inspects what a
  unit "really" is.
- **Progression metadata stays outside `Prompt`.** Prompt is presentation; stage
  ladders, prerequisites, and supersession live in separate queue/progression
  inputs.
- **Mastery is policy, not dogma.** Vault, Ruminatio, and Scry use different
  mastery thresholds. Progression helpers must accept an injected mastery
  predicate rather than hardcoding one global rule.
- **Queue primitives stay shallow.** They operate on canonical candidate data
  and recent-history keys, not consumer ORM documents.
- **`recitation` is deterministic.** It reuses the exact-answer/accepted-variant
  grading path and is explicitly not part of slice 3's rubric surface.

## Authority Order

tests > type system > code > docs > lore

## Repo Anchors

- `src/types.ts` — current canonical substrate for prompt/result/schedule types.
- `src/grader.ts` — existing deterministic grading surface that `recitation`
  extends.
- `src/scheduler.ts` — current pure FSRS wrapper and the shape slice 2 must
  preserve.
- `/Users/phaedrus/Documents/daybook/tools/vault-srs/src/queue.ts` — strongest
  current queue semantics: due-first, anti-clumping, prerequisites, supersession.
- `/Users/phaedrus/Documents/daybook/tools/vault-srs/tests/queue.test.ts` —
  executable queue oracle corpus to lift into kernel tests.
- `/Users/phaedrus/Development/ruminatio/convex/lib/progression.ts` — stage
  unlock logic and `lockedFreshCount` behavior for staged memorization.
- `/Users/phaedrus/Development/ruminatio/convex/scheduler.ts` — due-first burst
  selection and fresh round-robin that should remain app-owned unless proven
  otherwise.
- `/Users/phaedrus/Development/ruminatio/apps/web/app/study-system.test.ts` —
  recitation grading and stage-unlock fixtures.
- `/Users/phaedrus/Development/scry/convex/fsrs/engine.ts` — concept-level
  consumer mapping layer and proof that opaque review-unit identity matters.

## Prior Art

- Vault SRS queue semantics are the primary exemplar for progression-aware
  eligibility and next-card choice.
- Ruminatio is the primary exemplar for staged fresh-item unlocking and
  deterministic recitation grading.
- Scry is the primary exemplar for concept-level consumers that should still
  fit the same scheduler/progression substrate.

## Exemplar Techniques

- **Progression as data, not prompt shape** — Vault's `progressionGroup`,
  `requires`, and `supersedes` fields in
  `/Users/phaedrus/Documents/daybook/tools/vault-srs/src/types.ts`.
- **Conservative unlock gating with explicit user-facing counts** —
  Ruminatio's `filterUnlockedCandidates()` in
  `/Users/phaedrus/Development/ruminatio/convex/lib/progression.ts`.
- **Consumer-owned mapping layer at the boundary** — Scry's `stateToCard` /
  `cardToState` pair in
  `/Users/phaedrus/Development/scry/convex/fsrs/engine.ts`.

## Oracle (Definition of Done)

1. **Progression helpers reproduce both Vault and Ruminatio semantics via
   injected policy.**
   ```
   cd memory-engine && bun test tests/progression/
   ```
   This suite must cover:
   - Vault-style `requires` gating and `supersedes` burial.
   - Ruminatio-style later-stage lock/unlock behavior.
   - At least two distinct mastery predicates proving the API does not bake in
     one app's threshold.

2. **Queue primitives reproduce Vault's next-card behavior without absorbing
   session choreography.**
   ```
   cd memory-engine && bun test tests/queue/
   ```
   This suite must cover:
   - review-before-new ordering
   - same-source anti-clumping with graceful fallback
   - progression-aware suppression/unlock behavior
   - "show the locked thing rather than hide it forever" fallback

3. **Deterministic recitation grading lands as an additive prompt arm.**
   ```
   cd memory-engine && bun test tests/grader/recitation.test.ts
   ```
   The cases come from Ruminatio's existing recitation tests and must grade via
   the deterministic path, not a rubric adapter.

4. **Slice-2 fixtures are published through the public testkit surface.**
   ```
   cd memory-engine && bun test tests/testkit/slice2-fixtures.test.ts
   ```
   The exported corpora must include non-empty progression, queue, and
   recitation fixtures and run against live kernel behavior.

5. **Concept-level consumer canary stays green.**
   ```
   cd /Users/phaedrus/Development/scry && git switch memory-engine-canary && bun test
   ```
   The canary keeps Scry's own persisted state shape and maps to/from
   `ScheduleState` at the edge.

6. **Gate stays green.**
   ```
   cd memory-engine && bun run ci
   ```

## Implementation Sequence

1. Extend `src/types.ts` with progression and queue candidate metadata that is
   separate from `Prompt`.
2. Add `src/progression.ts` with injected-policy mastery/eligibility helpers and
   tests lifted from Vault + Ruminatio.
3. Add `src/queue.ts` with canonical next-card primitives and tests lifted from
   Vault's queue corpus.
4. Extend `Prompt`/`Grader` with a deterministic `recitation` arm and pin it
   with Ruminatio-derived fixtures.
5. Promote progression/queue/recitation corpora into `memory-engine/testkit`.
6. Wire a Scry canary branch through the kernel behind a consumer-owned mapping
   layer and iterate on the kernel until the canary is green.

## Risk + Rollout

- **Risk: queue primitives overfit Vault.** Mitigation: keep burst/session
  planning out of core, and validate against both Vault and Ruminatio fixtures.
- **Risk: mastery semantics drift between apps.** Mitigation: require injected
  mastery policy and test at least Vault + Ruminatio thresholds explicitly.
- **Risk: concept-level consumers pressure the type model.** Mitigation: make
  Scry the slice-2 canary and reject any design that inspects concept-specific
  fields inside core logic.
- **Risk: topology work crowds out semantics.** Mitigation: defer physical
  package split until slice 3 proves `adapters` is a real, durable surface.

Rollout:

- Ship slice 2 in the current single-package repo.
- Land the Scry canary as a branch/PR, not a forced migration.
- Revisit physical package split only after slice 3 if subpath exports start to
  creak under real adapter/versioning pressure.
