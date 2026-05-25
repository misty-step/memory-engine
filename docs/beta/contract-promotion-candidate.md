# Contract Promotion Candidate

Refs-backlog: 34

## Chosen Candidate

Promote the **compact review-state projection** semantics as the next
cross-client contract candidate:

- `ReviewStateProjection`: `{ due, reps, lapses, state, last_review }`
- `ScheduleChange`: `{ before, after }` using the same projection shape

This contract is now shared in `service/review-state-projection.ts` and consumed
by both beta clients.

## Cross-Client Evidence

Two independent workflows require the same semantics without client fields:

1. Mobile-first beta study (`experiments/beta-study/index.ts`) needs
   review-state projection and before/after schedule change display after
   `grade/apply-review`.
2. Command-first coach workflow (`experiments/beta-study/multi-client.ts`) needs
   the same projection and schedule-change semantics for command-trace output.

Pressure proof:

- `bun test experiments/beta-study/` verifies projection parity across clients.
- `bun test tests/service/` verifies accepted projection paths and rejected
  non-contract payloads (extra fields or invalid state values).

## Rejected Non-Promoted Alternatives

1. `reveal` command semantics: rejected because reveal remains display-only UI
   state with zero scheduling effects.
2. Activity metadata (`activityKind`, `activityStage`, worked solutions, rubric):
   rejected because these remain client-owned composition concerns.
3. Client idempotency-key format strings: rejected because prefixes remain
   workflow-specific (`beta-study:*` vs `beta-coach:*`).

## Promotion Readiness

**Promotion-ready as a service-level shared candidate now.**

**Not ready for `src/` package/export promotion yet.** We still need at least
one non-beta client or extraction boundary decision proving this DTO belongs in
the published kernel surface rather than repo-local service helpers.
