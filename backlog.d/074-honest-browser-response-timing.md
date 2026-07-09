# Record honest browser response time before grading

Priority: P0 · Status: ready · Estimate: S

## Goal

Stop the web review loop from fabricating a 1.8-second answer time and
automatically rating every mature correct answer `Easy`; scheduler inputs must
reflect the learner's actual effort.

## Oracle

- [ ] The rendered review form measures elapsed time from item presentation to
      answer submission and sends that value through the existing typed review
      boundary instead of the constant `1_800`.
- [ ] A mature correct answer taking more than four seconds maps to `Good` (3),
      while a genuinely fast mature correct answer can map to `Easy` (4), using
      the existing rating policy without weakening it.
- [ ] Missing, malformed, negative, and implausibly large client timing values
      have an explicit conservative policy and cannot manufacture `Easy`.
- [ ] A behavior test drives the real rendered form/route boundary; no internal
      collaborator is mocked.
- [ ] A phone-sized local browser walk records one slow-correct and one
      fast-correct attempt, including the resulting grade/rating evidence.
- [ ] `bun run ci` and `bun run ci:full` pass.

## Verification System

- Claim: the browser supplies an honest-enough effort signal for the existing
  deterministic rating policy.
- Falsifier: waiting more than four seconds still yields `Easy`, or malformed
  timing can force `Easy`.
- Driver: focused API/render route test plus a local phone-sized review walk.
- Grader: returned `GradeResult.rating` and persisted attempt timing, not a JS
  implementation detail.
- Evidence packet: focused test output and `docs/qa/074-browser-response-time.md`
  with request/result pairs and screenshots.
- Cadence: red before implementation, focused after each edit, fast/full gates
  before handoff.

## Notes

Evidence: `crates/memory-engine-api-render/src/render.rs` currently posts
`responseTimeMs=1800` for every browser answer, while the rating policy treats a
correct mature answer at or below four seconds as `Easy`. This corrupts both
scheduling and any later response-time analysis.
