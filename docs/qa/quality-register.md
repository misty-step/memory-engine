# Quality Improvement Register

Refs-backlog: 25

This register records quality opportunities found by QA. These are not all
bugs. Promote an item to `backlog.d/` when it becomes necessary for delivery,
consumer proof, or a shaped extraction decision.

## Active Opportunities

### QI-001: Consumer canary recertification is separate from package QA

- Dimension: consumer proof.
- Current evidence: `bun run qa` proves package behavior, service prototype
  behavior, dogfood experiments, and Dagger CI. It does not run Scry or Vault
  canary branches.
- Improvement: define a canary lane only when a ticket changes an exported
  contract used by Scry or Vault, with exact commands recorded beside the
  ticket.
- Promote when: exports, scheduling, grading, adapters, fixtures, progression,
  or queue semantics change in a way that claims consumer compatibility.

### QI-002: Benchmark receipts are non-gating

- Dimension: performance quality.
- Current evidence: `bun run bench` reports grader, scheduler, queue, and
  service throughput, but there is no historical budget.
- Improvement: capture benchmark receipts across several stable machines or CI
  containers, then shape a ticket for conservative regression budgets.
- Promote when: a repeatable slowdown appears in queue selection, grading, FSRS
  scheduling, or service command composition.

### QI-003: Evals cover stable deterministic semantics but not rubric model drift

- Dimension: semantic eval coverage.
- Current evidence: `tests/evals/regression-corpus.test.ts` replays deterministic
  grading, scheduling, progression, and queue fixtures through live API
  surfaces. Rubric grading is contract-tested with a static adapter, not a
  model-quality corpus.
- Improvement: add a small rubric eval corpus that records expected criterion
  outcomes and confidence handling for adapter implementations.
- Promote when: a real model-backed rubric adapter or consumer canary depends on
  rubric quality beyond the current static contract.

### QI-004: Dogfood clients expose repeated view-state pressure

- Dimension: API ergonomics.
- Current evidence: CLI review, import probe, and web shell all keep session
  choreography outside `src`. The web shell also records pressure for a compact
  review-state DTO and reveal semantics.
- Improvement: compare repeated pressure in `docs/dogfood/extraction-decision.md`
  before promoting view DTOs, reveal commands, or helper APIs.
- Promote when: a second interactive client independently needs the same
  review-state projection or reveal behavior.

### QI-005: QA harness reports lane receipts but does not persist run artifacts

- Dimension: auditability.
- Current evidence: `bun run qa` prints deterministic lane names, commands,
  elapsed time, and pass/fail receipts to stdout.
- Improvement: add an explicit `--report <path>` mode that writes a timestamped
  Markdown or JSON receipt under a non-source artifact directory.
- Promote when: review workflow needs comparable QA receipts across branches or
  long-running cycles.
