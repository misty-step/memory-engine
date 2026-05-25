# Boundary Reshape Trigger Check

Refs-backlog: 36

## Purpose

Define explicit, measurable conditions for moving ownership across the current
kernel/service/application boundary, then score current beta evidence against
those conditions.

## Trigger Criteria

Boundary reshaping is allowed only when every trigger below passes.

| Trigger | Pass threshold | Why this is load-bearing |
| --- | --- | --- |
| Multi-consumer breadth | At least 3 independent consumers, including at least 1 non-`experiments/beta-study` consumer, require the same new shared ownership surface without per-client forks. | Prevents reshaping on one-workflow local pressure. |
| Repeated durability | The same cross-consumer need stays stable for 2 consecutive ticket cycles with executable receipts each cycle. | Prevents single-cycle overfitting. |
| Boundary failure pressure | At least 2 distinct boundary-caused failures or correctness regressions are reproduced with tests and traced to current ownership split. | Requires proof that current boundary is harming correctness, not preference. |
| Neutral promotion candidate | A provider-neutral and persistence-neutral contract can be named with exact fields/commands, and all consumers above adopt it unchanged. | Preserves `src/` purity and prevents app details from leaking inward. |
| Migration scope clarity | A single shaped ticket can enumerate exact file-scope moves, non-goals, and oracle commands proving parity after the move. | Prevents open-ended architecture drift. |

## Current Evidence Scorecard

| Trigger | Evidence | Result |
| --- | --- | --- |
| Multi-consumer breadth | `docs/beta/multi-client-pressure.md` proves 2 clients (mobile + coach) inside `experiments/beta-study`; no non-beta consumer requires a new shared ownership surface. | Fail |
| Repeated durability | `docs/beta/extraction-decision.md` and `docs/beta/extract-beta-app-readiness.md` show one current pressure cycle; no second consecutive cycle with a new reshape need. | Fail |
| Boundary failure pressure | `docs/beta/multi-client-pressure.md` and `docs/beta/mobile-study.md` report boundary holds (reveal UI-owned, stable submit semantics, resume works). No reproduced correctness failures caused by ownership split. | Fail |
| Neutral promotion candidate | `docs/beta/contract-promotion-candidate.md` identifies `ReviewStateProjection`/`ScheduleChange` as service-level shared candidate, but explicitly not ready for `src/` promotion. | Partial (fails trigger) |
| Migration scope clarity | No shaped boundary-change ticket exists because gating triggers are not met. | Fail |

## Outcome

Decision: **keep boundary as-is**.

Current receipts continue to support the existing split:

- `src/`: pure kernel semantics only.
- `service/`: local command boundary (`next-queue`, `grade/apply-review`) and
  shared review-state projection helpers.
- `experiments/`: persistence, generation, reveal UI state, and client
  choreography.

No boundary-change ticket should be shaped until all trigger criteria pass.

