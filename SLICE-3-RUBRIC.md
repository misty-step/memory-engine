---
shaping: true
slice: 3
parent: SPEC.md
date: 2026-04-17
status: shaped-awaiting-slice-2-completion
---

# Context Packet: Memory Engine — Slice 3 (Rubric Contract + Adapter Surface)

## Goal

Add rubric-based grading to the kernel through an additive async surface and a
vendor-neutral adapter contract, while preserving the existing synchronous
deterministic `Grader` and proving the boundary on a Vault canary.

## Non-Goals

- No recitation work. Recitation is deterministic and belongs to slice 2.
- No vendor SDK implementation in the repo. Vault or another consumer keeps its
  OpenAI/Anthropic client locally behind the shared adapter interface.
- No package split. `adapters` starts as a logical surface and can ship as a
  subpath export inside the existing package.
- No retries, rate limiting, caching, or cost controls in core.
- No streaming or partial-grade protocol in v1.

## Constraints / Invariants

- **Core stays pure.** `src/` may define rubric contracts and async grading
  orchestration, but it may not import network clients or vendor SDKs.
- **The synchronous deterministic API remains stable.** Existing consumers of
  `new Grader().grade(...)` must keep working unchanged.
- **Rubric support is additive.** Introduce a new async grading surface rather
  than making the current deterministic `Grader` promise-returning.
- **One result envelope.** Rubric grading still resolves to the same canonical
  `GradeResult` shape, augmented only with additive rubric fields.
- **Adapter failures throw.** A transport/model failure is not silently mapped to
  `revealed`; consumers handle retries and fallback policy.
- **Confidence/rubric mapping is explicit.** Required criteria, passing score,
  and confidence threshold are pinned by tests, not left to prose.

## Authority Order

tests > type system > code > docs > lore

## Repo Anchors

- `src/types.ts` — current prompt/result substrate that slice 3 extends.
- `src/grader.ts` — current sync deterministic surface that slice 3 must not
  break.
- `src/index.ts` — current public exports and the place additive surfaces must be
  wired carefully.
- `/Users/phaedrus/Documents/daybook/tools/vault-srs/src/grading.ts` — current
  rubric outcome mapping, confidence floor, and criterion handling.
- `/Users/phaedrus/Documents/daybook/tools/vault-srs/src/rubric-grader.ts` —
  current adapter boundary and criterion normalization logic.
- `/Users/phaedrus/Documents/daybook/tools/vault-srs/src/types.ts` — current
  rubric type shapes (`RubricDefinition`, `RubricCriterionResult`).
- `/Users/phaedrus/Documents/daybook/tools/vault-srs/tests/grading.test.ts` —
  executable rubric oracle cases to lift.
- `testkit/fixtures.ts` — existing contract-fixture publication point that slice
  3 extends.

## Prior Art

- Vault SRS already has the complete local rubric pipeline: rubric prompt
  metadata, criterion result normalization, confidence thresholding, and a local
  OpenAI-backed adapter.
- Slice 1's deterministic grader is the compatibility boundary that must remain
  stable.

## Exemplar Techniques

- **Criterion-by-name normalization before trusting model output** —
  `normalizeAssessment()` in
  `/Users/phaedrus/Documents/daybook/tools/vault-srs/src/rubric-grader.ts`.
- **Conservative confidence floor with required-criterion gating** —
  `gradeRubric()` in
  `/Users/phaedrus/Documents/daybook/tools/vault-srs/src/grading.ts`.
- **Keep prompt engineering out of core** — Vault's local OpenAI request builder
  is a consumer concern, not a kernel concern.

## Oracle (Definition of Done)

1. **Rubric outcome mapping is pinned by executable contract tests.**
   ```
   cd memory-engine && bun test tests/grader/rubric-contract.test.ts
   ```
   The suite must cover:
   - all-required + passing-score + high-confidence → `correct`
   - partial pass or low confidence → `close`
   - no meaningful rubric pass → `wrong`
   - adapter failure → throw

2. **The additive async surface does not break synchronous deterministic callers.**
   ```
   cd memory-engine && bun test tests/grader/async-surface.test.ts
   ```
   The suite must prove:
   - deterministic prompts still use the existing synchronous `Grader`
   - rubric prompts grade through the new async surface
   - typecheck catches misuse rather than widening everything to `Promise`

3. **Rubric fixtures ship through the public testkit.**
   ```
   cd memory-engine && bun test tests/testkit/rubric-fixtures.test.ts
   ```

4. **Adapter interfaces and test doubles are published as a separate surface.**
   ```
   cd memory-engine && bun test tests/adapters/rubric-adapter.test.ts
   ```
   and
   ```
   cd memory-engine && bun run typecheck
   ```
   must pass with consumers that never import the adapter subpath.

5. **Vault rubric canary stays green.**
   ```
   cd /Users/phaedrus/Documents/daybook/tools/vault-srs && git switch memory-engine-rubric-canary && bun test
   ```
   Vault keeps its local OpenAI implementation, but it implements the shared
   memory-engine adapter interface and uses the shared rubric result contract.

6. **Gate stays green.**
   ```
   cd memory-engine && bun run ci
   ```

## Implementation Sequence

1. Extend `src/types.ts` with rubric prompt/result types and additive rubric
   fields on `GradeResult`.
2. Add an async rubric grading surface that composes with, but does not replace,
   the existing synchronous deterministic `Grader`.
3. Publish vendor-neutral rubric adapter interfaces and test doubles under a new
   adapters surface.
4. Lift Vault rubric fixtures and confidence-threshold cases into kernel tests
   and `memory-engine/testkit`.
5. Wire Vault's local OpenAI grader to the shared interface on a
   `memory-engine-rubric-canary` branch and iterate on the kernel until green.

## Risk + Rollout

- **Risk: async surface infects all deterministic callers.** Mitigation: add a
  new async surface; keep `Grader` sync and stable.
- **Risk: vendor assumptions leak into core.** Mitigation: ship only shared
  contracts/test doubles; leave real model calls in consumers.
- **Risk: confidence semantics drift from Vault.** Mitigation: lift Vault's
  rubric cases verbatim into contract tests before changing code.
- **Risk: adapter surface justifies a package split too early.** Mitigation:
  start with a subpath export in the current package; split only after real
  multi-surface pressure appears.

Rollout:

- Land slice 3 behind additive exports and new tests.
- Validate first on a Vault canary branch with the consumer's existing model
  pipeline.
- Revisit physical package split only after slice 3 if `adapters` proves to be a
  durable, independently versioned surface.
