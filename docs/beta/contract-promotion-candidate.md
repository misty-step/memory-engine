# Contract Promotion Candidate


## Chosen Candidate

Promote the **compact review-state projection** semantics as the next
cross-client contract candidate:

- `ReviewStateProjection`: `{ due, reps, lapses, state, last_review }`
- `ScheduleChange`: `{ before, after }` using the same projection shape

This contract is now represented by the Rust study/service DTOs and consumed by
the beta app through `crates/memory-engine-study`.

## Cross-Client Evidence

Two independent workflows require the same semantics without client fields:

1. Mobile-first beta study (`crates/memory-engine-study`) needs
   review-state projection and before/after schedule change display after
   `grade/apply-review`.
2. Local Rust HTTP workflow (`crates/memory-engine-beta-app`) needs the same
   projection and schedule-change semantics for browser/API output.

Pressure proof:

- `cargo test -p memory-engine-study -p memory-engine-beta-app` verifies
  projection behavior across session and HTTP boundaries.
- `cargo test -p memory-engine-service` verifies service command semantics.

## Rejected Non-Promoted Alternatives

1. `reveal` command semantics: rejected because reveal remains display-only UI
   state with zero scheduling effects.
2. Activity metadata (`activityKind`, `activityStage`, worked solutions, rubric):
   rejected because these remain client-owned composition concerns.
3. Client idempotency-key format strings: rejected because prefixes remain
   workflow-specific (`beta-study:*` vs `beta-coach:*`).

## Promotion Readiness

**Promotion-ready as a service-level shared candidate now.**

**Not ready for pure-kernel promotion yet.** We still need at least one non-beta
client or extraction boundary decision proving this DTO belongs in
`crates/memory-engine-core` rather than repo-local service/study helpers.
