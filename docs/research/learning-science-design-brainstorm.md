# Learning Science Design Brainstorm

This brainstorm translates the research reference pack into concrete
`memory-engine` design pressure. It is intentionally experimental; promote only
what repeated clients and evals prove.


## Design Principles

1. **Attempt-first learning.** A learner action is the atomic event. The system
   should prefer retrieval, generation, recitation, or explanation before
   reveal/re-read.
2. **Difficulty should be desirable, not punitive.** Make retrieval effortful,
   but avoid turning missing prerequisite knowledge into unproductive failure.
3. **Spacing is policy, not a fixed interval table.** Keep the scheduler pure
   and replayable, and preserve event data for future parameter fitting.
4. **Interleave for contrast.** Mix related confusable concepts; do not blindly
   shuffle unrelated domains.
5. **Feedback should be specific and staged.** Correctness alone is too thin.
   Clients need hints, error diagnosis, expected answer, rubric evidence, and
   delayed reflection modes.
6. **Calibration is part of mastery.** High confidence wrong answers and low
   confidence right answers need different follow-up than ordinary misses.
7. **Guidance should fade.** Novice workflows can start with worked examples or
   cues, then progress toward free recall and transfer.
8. **Dogfood before abstraction.** If one experiment needs a helper, keep it
   local. If two independent clients need it, shape a stable API/testkit ticket.

## Experimental Clients

### Calibration CLI

A terminal loop that asks for confidence before or after each answer, then emits
attempt, grade, response time, schedule update, and calibration error.

Pressure tested:

- optional confidence/prediction metadata
- calibration eval helpers
- end-of-session reflection receipts

### Interleaving Inspector

A local web shell that shows the next prompt and a "why this item" trace:
due, locked, buried, prerequisite, anti-clumped, repair, or fresh.

Pressure tested:

- queue explainability as debug/eval output
- concept/source/domain metadata quality
- whether queue policies are comprehensible to clients

### Pretest Mode

A diagnostic client that asks before teaching. Wrong answers create repair
tasks or progression edges, then the client offers a worked example and retest.

Pressure tested:

- unknown-material workflows
- misconception repair fixtures
- boundary between content authoring and review-unit creation

### Recitation Studio

A narrow memorization client for prayers, poems, speeches, proofs, or passages.
Stages: worked example, gist recall, cloze, first-letter cue, full recitation,
delayed recitation.

Pressure tested:

- progression ladders
- recitation grading
- cue fading
- reveal-is-not-mastery semantics

### Authoring Diff Tool

Paste a small markdown/JSON fixture and inspect the normalized `Prompt`,
`QueueCandidate`, progression edges, and rejected product-owned fields.

Pressure tested:

- import-probe boundaries
- content adapter conformance
- what belongs in `testkit` versus clients

### Study Replay Viewer

Load a fixture session and render the timeline: attempt, confidence, grade,
feedback, schedule transition, queue decision, and reflection checkpoint.

Pressure tested:

- event schema completeness
- benchmark/eval receipts
- debugging and docs for API consumers

### Rubric Duel

Run deterministic grading, static rubric grading, and future model-backed
rubric grading over the same corpus; compare verdicts, confidence, feedback,
and criterion evidence.

Pressure tested:

- adapter contract thickness
- rubric evals
- model-provider-free baselines

### Failure Lab

A tiny CLI or web client that intentionally fails store calls: attempt write,
schedule apply, mismatched review unit, blank answer, duplicate attempt.

Pressure tested:

- service boundary failure semantics
- realistic fake stores
- safe client error handling

## API And Eval Opportunities

### Learning-Semantic Eval Corpus

Named scenarios that run through live APIs:

- `near-miss-is-close`
- `reveal-does-not-master`
- `failed-recall-needs-feedback`
- `prerequisite-unlocks-next-stage`
- `superseded-easy-stage-is-buried`
- `interleave-confusable-concepts`
- `anti-clump-yields-to-urgent-review`
- `low-confidence-correct-needs-recheck`
- `high-confidence-wrong-triggers-repair`
- `worked-example-fades-to-recall`

### Queue Explanation Trace

Start as experiment/test output, not public API. Shape only if multiple clients
need stable explainability.

Candidate trace fields:

- candidate id
- due status
- priority bucket
- progression decision
- separation pass
- selected/rejected reason

### Calibration Metrics

Candidate eval helpers:

- confidence error
- overconfidence rate
- low-confidence correct rate
- confidence shift after feedback
- retention after calibration review

### Session Recipe Fixtures

Keep recipes in testkit only if they stay content-neutral:

- vocabulary drill
- memorization ladder
- concept repair
- translation/gist
- cumulative exam prep
- rubric explanation

### Workflow Benchmarks

Measure whole loops rather than only isolated functions:

- grade/apply-review + next queue over 10, 100, 1,000 candidates
- progression filtering over deep stage ladders
- anti-clumping with large recent history
- rubric normalization over large criterion sets

## Issue Queue Pressure

Strong candidates for future tickets after the current Slice 5 set:

- Schedule strategy/version boundary and dry-run migration simulation.
- Attempt metadata expansion for confidence, reveal mode, and feedback mode.
- Queue explanation traces for eval/debug output.
- Learning-semantic eval corpus with named scenarios above.
- Recitation Studio dogfood after CLI/import prove the basic fixture path.
- Calibration CLI as either part of ticket 21 or a follow-up ticket if it would
  overload the first CLI review loop.

## Design Guardrails

- Do not put authored content taxonomy in `crates/memory-engine-core`.
- Do not promote a UI workflow to API after one client.
- Do not make feedback timing a kernel constant.
- Do not treat confidence as correctness.
- Do not let model-backed rubric experiments replace deterministic baselines.
- Do not optimize only for short-term correct answers; optimize for delayed
  retention, transfer, calibration, and workload.
