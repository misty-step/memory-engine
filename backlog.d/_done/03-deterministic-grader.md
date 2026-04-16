---
shaping: true
ticket: 03-deterministic-grader
slice: 1
status: shipped
priority: high
estimate: M
depends_on: [01-canonical-types]
oracles:
  - bun run ci
  - bun test tests/grader/
---

# Deterministic grader (mcq / boolean / cloze / shortAnswer) with injected rating policy

## Goal

A single `Grader.grade(prompt, submitted, ctx) → GradeResult` entry point
that covers the four deterministic `Prompt` arms with Vault SRS's grading
semantics (Levenshtein thresholds, accepted variants, equivalence groups,
ignored tokens) and returns a `GradeResult` with `rating` already
populated by an injected rating policy (default = Ruminatio's
fast+reps≥3 heuristic).

## Non-Goals

- No rubric grading (slice 3).
- No LLM / AI-assisted grading (slice 3).
- No recitation prompt arm (slice 3 or later).
- No redesign of Vault's Levenshtein thresholds — lift verbatim.
- No pluggable comparator (per bench review: N=1, premature).
- No two-call protocol. `grade()` returns rating already populated. Do
  not expose a separate `toRating` function on the public API.
- No storage access, no side effects, no logging.

## Oracle

- [ ] `bun test tests/grader/parity.test.ts` passes. Corpus: the four
      deterministic fixture behaviors lifted from
      `/Users/phaedrus/Documents/daybook/tools/vault-srs/tests/grading.test.ts`
      plus the corresponding deterministic branches in
      `/Users/phaedrus/Documents/daybook/tools/vault-srs/src/grading.ts`
      (exclude `short-answer-rubric` entries — slice 3). For each
      fixture: call `Grader.grade(prompt, submitted, ctx)`; assert the
      resulting `GradeResult` matches the canonical kernel envelope
      byte-for-byte (`verdict`, `rating`, `isCorrect`, `submittedAnswer`,
      `expectedAnswer`, `graderKind`, `graderModel`,
      `graderConfidence`, `feedback`).
- [ ] `bun test tests/grader/rating-policy.test.ts` passes. Table test:
      all combinations of `verdict ∈ {correct, close, wrong, revealed}`
      × `fast ∈ {responseTimeMs=3000, responseTimeMs=10000}` ×
      `reps ∈ {0, 3, 10}`. Assert the default rating policy produces:
      `correct` + fast + `reps ≥ 3` → `4`;
      `correct` otherwise → `3`;
      `close` → `2`;
      `wrong | revealed` → `1`.
- [ ] `bun test tests/grader/exhaustiveness.test.ts` passes. Uses
      `// @ts-expect-error` to confirm that removing any arm from the
      grader's `switch (prompt.kind)` breaks compilation.
- [ ] `bun run ci` exits 0.

## Notes

- Implementation: `src/grader.ts`.
- Public API: `Grader.grade(prompt: Prompt, submitted: string, ctx:
  GradeCtx): GradeResult`. `GradeCtx` is `{ responseTimeMs: number;
  priorReps: number }`.
- Rating policy: `type RatingPolicy = (verdict: Verdict, ctx: GradeCtx)
  => Rating`. Exported constant `defaultRatingPolicy: RatingPolicy`
  implements Ruminatio's mapping.
- Policy injection: constructor `new Grader({ ratingPolicy?:
  RatingPolicy })`. Default constructor wires `defaultRatingPolicy`.
- Dispatcher: `switch (prompt.kind)` with `assertNever(prompt)` in
  `default`. Each arm calls a small per-form helper or the shared exact
  grader for `cloze` / `shortAnswer`.
- Canonical slice-1 output shape comes from ticket 01: `expectedAnswer`
  is the public field name and there is no comparison envelope in
  `GradeResult`. Earlier shaping text that mentioned those fields was
  superseded when 01 landed.
- Study (and lift from)
  `/Users/phaedrus/Documents/daybook/tools/vault-srs/src/grading.ts`:
  - `normalize(answer: string): string` — lowercase, strip diacritics,
    collapse whitespace.
  - Levenshtein distance thresholds: `0` for answer length ≤ 4, `1` for
    ≤ 8, `2` otherwise.
  - Accepted-variant substitution: try every `acceptedAnswers[i]` under
    normalize+distance.
  - Equivalence groups: within each group, any member matches any other
    member.
  - Ignored tokens: stripped from both submitted and canonical before
    comparison.
- Rubric fixtures in Vault's corpus — filter out `short-answer-rubric` at
  fixture-load time. If a rubric fixture leaks in, the dispatcher will
  fail to match the Prompt kind. That's the correct failure mode; test
  should not paper over it.
- Ticket 04 (testkit) will hoist the fixture corpus to a canonical
  location. For this ticket, keep a local copy under
  `tests/grader/fixtures.ts` — ticket 04 replaces it.

## Parallelism

Can proceed in parallel with 02 once 01 is merged. No cross-dependency.

## What Was Built (cycle 01KPC6Z342T1YT2T2GYVJMK5B0)
- Branch: feat/03-deterministic-grader
- Evidence: backlog.d/_cycles/01KPC6Z342T1YT2T2GYVJMK5B0/evidence/
