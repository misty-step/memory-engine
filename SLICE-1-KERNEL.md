---
shaping: true
slice: 1
parent: SPEC.md
date: 2026-04-16
status: ready-for-implement
---

# Context Packet: Memory Engine — Slice 1 (Kernel v0.1)

## Goal

Extract the narrowest useful shared kernel — canonical types, FSRS reference
scheduler, and deterministic grading for four prompt forms — as a single
TypeScript package, and prove it by landing a Vault SRS branch that imports
the kernel and keeps all existing Vault tests green.

## Non-Goals

- Progression graph primitives (`requires`, `supersedes`, `stageOrder`).
  Deferred to slice 2.
- Queue selection and anti-clumping. Deferred to slice 2.
- Rubric and AI-assisted grading. Deferred to slice 3 behind an adapter
  contract.
- Session choreography, content parsing, storage adapters, authoring
  pipelines. Out of scope entirely for the kernel.
- Four-package monorepo split (`contracts` / `core` / `adapters` / `testkit`).
  Deferred until slice 2 introduces the adapter boundary. Slice 1 ships as
  one package with subpath exports (`memory-engine`, `memory-engine/testkit`).
- `RatingMapper` as a separate interface. Verdict → Rating is a function
  parameter on the grader with a default policy, not a pluggable module.
- Separate `Grader.grade` + `RatingMapper.toRating` two-call protocol.
- Camel-case translation of `ScheduleState`. The shape matches ts-fsrs
  `Card` verbatim; translation deferred until a scheduler swap forces it.

## Constraints / Invariants

- **Core is pure.** No Convex, React, Hono, Bun, Node, or framework imports
  in the runtime path. Time, storage, and identity are injected.
- **Consumer owns persistence.** The kernel receives `ScheduleState` as an
  argument and returns the next `ScheduleState`. It never reads or writes.
- **`ScheduleState` matches ts-fsrs `Card` exactly** (snake_case,
  `state: 0|1|2|3`, `last_review: number | null`). This is a deliberate
  leak; document it as a first-class invariant. Consumers must not assume
  it will survive a scheduler swap.
- **Prompt ↔ Grader co-evolve.** Adding a new `Prompt` arm requires a
  concurrent grader dispatch update and an `assertNever` exhaustiveness
  check. No consumer may ship a new prompt form without both.
- **One grade call, one result.** `Grader.grade()` returns a `GradeResult`
  with `rating` already populated by the injected rating policy. There is
  no two-step verdict-then-rating protocol across the module boundary.
- **Verdict vocabulary is fixed:** `'correct' | 'close' | 'wrong' | 'revealed'`.
  Ruminatio's `partial` is a rename (→ `close`); Caesar's SCREAMING case is a
  rename; no semantic migration.
- **Authority order:** tests > type system > code > docs > lore.

## Repo Anchors

These files define patterns the kernel must match or absorb. Study before
writing code.

- `../../Documents/daybook/tools/vault-srs/src/grading.ts` — deterministic
  grader blueprint. The Levenshtein thresholds (0/1/2 by length),
  accepted-variant substitution, equivalence groups, and ignored-token
  stripping are lifted whole. **Do not redesign.**
- `../../Documents/daybook/tools/vault-srs/src/types.ts` — strongest existing
  canonical type shape. `GradeResult`, `ItemType`, `FsrsCardState` are the
  reference.
- `../../Documents/daybook/tools/vault-srs/tests/grading.test.ts` — the
  fixture corpus the kernel must reproduce bit-exactly.
- `../ruminatio/convex/lib/grading.ts` — rating policy reference.
  The default `ratingPolicy` implements: `correct + responseTimeMs ≤ 4000 +
  priorReps ≥ 3 → 4`; `correct → 3`; `close → 2`; `wrong | revealed → 1`.
- `../ruminatio/convex/scheduler.ts` — FSRS wrapper pattern. The kernel's
  `Scheduler.next(state, rating, now)` collapses this file's surface area.
- `../caesar-in-a-year/lib/srs/fsrs.ts` — simplest FSRS wrapper in the
  portfolio; confirms the kernel's one-function scheduler surface is
  sufficient.
- `../scry/convex/fsrs/engine.ts` — validates that concept-level consumers
  can sit on the same `ScheduleState` as phrasing-level consumers (the
  `ReviewUnitId` opacity is the key).

## Prior Art

- ts-fsrs: <https://github.com/open-spaced-repetition/ts-fsrs>. Reference
  scheduler. Used by all four portfolio apps. Kernel wraps, does not
  reimplement.
- Vault SRS `grading.ts` is the de-facto engine spec inside the portfolio.
  Treat divergence from its semantics as a bug, not a design choice, for
  slice 1.

## Oracle (Definition of Done)

Slice 1 is done when **all of the following** are true. Every item is a
command that returns pass/fail, not prose.

1. **Grading parity with Vault SRS.**
   Vault's `tests/grading.test.ts` fixture corpus runs against the kernel's
   `Grader.grade()` and produces identical `GradeResult` envelopes (modulo
   the documented field renames: `outcome` → `verdict`; no other drift).
   ```
   cd memory-engine && bun test tests/grading-parity.test.ts
   ```

2. **FSRS scheduler round-trip.**
   An empty state scheduled with `Rating.Good` at `t=0`, serialized to
   JSON, deserialized, and scheduled again with `Rating.Good` at `t=+1d`
   produces a `ScheduleState` byte-identical to the pure-ts-fsrs
   equivalent.
   ```
   cd memory-engine && bun test tests/fsrs-roundtrip.test.ts
   ```

3. **Default rating policy matches Ruminatio.**
   Table test covers the four verdicts × fast/slow × low/high reps matrix
   and produces Ruminatio's exact rating outputs.
   ```
   cd memory-engine && bun test tests/rating-policy.test.ts
   ```

4. **`ScheduleState` JSON round-trips lossless** for all four FSRS states
   (new / learning / review / relearning).
   ```
   cd memory-engine && bun test tests/schedule-state-serialize.test.ts
   ```

5. **Type check clean.**
   ```
   cd memory-engine && bun run typecheck
   ```

6. **Vault SRS canary green.**
   On a `memory-engine-canary` branch of Vault SRS, the grader and
   scheduler modules import from `memory-engine` (via local path or
   workspace), and `bun test` passes without modification to any other
   Vault test file.
   ```
   cd ../../Documents/daybook/tools/vault-srs && \
     git switch memory-engine-canary && bun test
   ```

If any oracle command exits non-zero, slice 1 is not done.

## Implementation Sequence

1. **Scaffold package.** `memory-engine/` with `package.json`, `tsconfig.json`,
   `src/`, `tests/`, `testkit/`. Single package, not a monorepo. Bun as the
   runtime/test driver (matches Vault). Export subpaths: `memory-engine`
   and `memory-engine/testkit`.
2. **Write contract types.** `src/types.ts`: `Prompt`, `Verdict`, `Rating`,
   `ScheduleState`, `GradeResult`, `ReviewUnitId`, `GraderKind`. Verbatim
   lifts from Vault's `types.ts` with documented renames.
3. **Port deterministic grader.** `src/grader.ts`. Lift the Levenshtein
   threshold logic, normalizer, accepted-variant and equivalence handling
   from Vault's `grading.ts`. Dispatch on `Prompt` discriminator with
   `assertNever` exhaustiveness. Add the `ratingPolicy` parameter (default =
   Ruminatio's policy).
4. **Write scheduler wrapper.** `src/scheduler.ts`. One function:
   `next(state: ScheduleState | null, rating: Rating, now: number):
   ScheduleState`. Internally instantiates `ts-fsrs` with
   `{ request_retention: 0.9, maximum_interval: 36500 }` and calls
   `repeat(card, now)[rating]`.
5. **Ship testkit.** `testkit/fixtures.ts`. Export Vault's grading fixture
   array and a small FSRS round-trip fixture array. These are consumed by
   slice-1 tests and will be consumed by slice-2 consumer contract tests.
6. **Write oracle tests 1–5.** Red → green. No implementation changes after
   tests lock behavior.
7. **Canary.** On a Vault SRS branch, replace `src/grading.ts` internals
   with a call into `memory-engine`. Replace FSRS scheduling in Vault's
   review application with `memory-engine`'s `scheduler.next`. Run Vault's
   full test suite. Iterate on the kernel (not on Vault tests) until green.
   Oracle 6 passes when the canary branch is green.

## Risk + Rollout

### Risks

- **Vault fixture drift.** Vault's grading fixtures may encode an
  implementation detail that the kernel rejects as a bug. Mitigation:
  treat a fixture mismatch as a diff-then-decide — either fix the kernel
  to preserve Vault's behavior, or fix the Vault fixture and note the
  behavior change in the canary PR.
- **Hidden rubric dependency.** Vault's `gradeSubmission` takes a
  `GradingServices` with an optional `rubricGrader`. The kernel drops this
  for slice 1. Mitigation: the canary branch only exercises the four
  deterministic prompt forms; `short-answer-rubric` items in Vault stay on
  Vault's existing path until slice 3.
- **FSRS parameter drift.** `request_retention: 0.9` is set in three of
  four portfolio apps but not audited in Scry. Confirm during slice 2
  adoption; no action required for slice 1.
- **ts-fsrs version skew.** Kernel pins a specific ts-fsrs version; Vault's
  current pin may differ. Mitigation: align Vault to the kernel's pin on
  the canary branch; capture any behavioral diff in a fixture.

### Rollout

- Kernel ships as a path dependency (Bun workspace or relative import)
  before any npm publish. No registry publication in slice 1.
- Vault's canary lands as a PR, not a merge-to-master, until slice 2 confirms
  the boundary holds.
- Zero production migration in slice 1. No persisted data shape changes;
  `ScheduleState` is JSON-compatible with Vault's existing `FsrsCardState`.

### Undo

- Revert the Vault canary branch. Kernel is additive; no other consumer
  depends on it.
- Kernel package is deletable with `rm -rf memory-engine/` since nothing
  external imports it until the canary lands.

## Open Questions Deferred to Slice 2

(Not blocking slice 1. Listed here so implementers don't try to answer
them now.)

- `ReviewUnit` normalization shape — deferred until progression enters scope.
- Queue policy core-vs-consumer split — deferred until queue enters scope.
- AI-grading adapter contract — deferred to slice 3.
- Polyglot consumer support — deferred until a non-TypeScript consumer is
  named.
