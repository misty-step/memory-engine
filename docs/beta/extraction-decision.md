# Beta Extraction Decision

Refs-Powder: memory-engine-031

## Primary Decision

Primary path: **keep experimenting**.

Keep the beta interface, persistence spine, generation probe, and hardened
service contract in `experiments/` and `docs/beta/` for one more pressure cycle.
Do not extract a standalone beta app, do not promote new package/API surfaces,
and do not reshape kernel/service ownership yet.

## Evidence Comparison

| Evidence lane | Current signal | Decision pressure |
| --- | --- | --- |
| CLI review (`docs/dogfood/cli-review.md`) | `grade/apply-review` + `next-queue` work end to end with calibration receipts. | Confirms core service commands; does not prove reusable app surface. |
| Import probe (`docs/dogfood/import-probe.md`) | Canonical types absorb authored fixture output without kernel changes. | Confirms content normalization can stay client-owned. |
| Web shell (`docs/dogfood/web-shell.md`) | Reveal is UI-owned; review-state projection needs compact client DTOs. | One interactive client is not enough to freeze promoted contracts. |
| Beta persistence (`docs/beta/persistence-spine.md`) | Durable restart/reload, atomic writes, duplicate protection are proven. | Strong beta-usability signal; still app-layer store ownership. |
| Beta generation (`docs/beta/content-generation.md`) | Provenance-backed quiz/exercise drafts with acceptance/rejection receipts. | Contract is testable, but provider-quality evals are still missing. |
| Mobile study (`docs/beta/mobile-study.md`) | Phone-size loop, resume, reveal, submit, and duplicate-submit protection are proven. | Useful local beta loop exists, but only one interface/workflow. |
| Service contract (`docs/beta/service-contract-v0.md`) | Stable local command lifecycle, typed failures, reveal policy. | Contract is stable enough for local beta, not yet cross-client-promotable. |
| Graduated ladder (`docs/beta/graduated-activity-ladder.md`) | Quiz/exercise ladder and progression unlocks are proven with provenance. | Stage vocabulary and ladder authoring are still beta-owned conventions. |

## Rejected Primary Paths (For Now)

1. **Extract beta app now**: rejected because evidence is from one in-repo
   interface and deterministic fixtures, not repeated independent clients.
2. **Promote helper/API contract now**: rejected because DTO pressure has not
   repeated across multiple clients with stable semantics.
3. **Reshape kernel/service boundary now**: rejected because no evidence shows
   kernel ownership of persistence/provider/UI concerns improves outcomes.

## Follow-Up Tickets Created

Selected path (keep experimenting):

- `33-multi-client-beta-pressure.md`

Rejected high-value alternatives to revisit with explicit gates:

- `34-promote-cross-client-contract-candidate.md`
- `35-extract-beta-app-readiness-gate.md`
- `36-reshape-boundary-trigger-check.md`

## Revisit Triggers

Revisit this decision when both are true:

1. At least two independent beta workflows/clients need the same service DTO
   and reveal/attempt semantics.
2. Those semantics remain provider-neutral and persistence-neutral under real
   repeated dogfood receipts (not single-fixture success only).
