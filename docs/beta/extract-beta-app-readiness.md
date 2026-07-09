# Extract Beta App Readiness Gate

Refs-Powder: memory-engine-035

## Decision

Outcome: **hold extraction after Rust cutover**.

Keep the beta study interface in the Rust workspace for the current cycle:
`crates/memory-engine-study`, `crates/memory-engine-persistence`,
`crates/memory-engine-generation`, and `crates/memory-engine-beta-app`.
Extraction remains a high-value option, but current evidence is still better
used to stabilize contracts in-repo before introducing cross-repo release
overhead.

## In-Repo vs Extracted-Repo Comparison

| Dimension | In-repo operation (today) | Extracted-repo cost (today) | Gate result |
| --- | --- | --- | --- |
| QA coverage | One gate (`bun run ci`) plus focused Rust beta oracles (`cargo test -p memory-engine-study -p memory-engine-beta-app`) verifies kernel, service, and beta clients together. | Requires split CI plus explicit cross-repo contract matrix to prevent drift between app and kernel versions. | Hold |
| Release friction | No package publish choreography needed for beta iteration; kernel and beta updates can land in one ticket. | Requires version pinning/publish cadence (or workspace coupling) before each boundary change. | Hold |
| Ownership clarity | Current boundary is explicit: `memory-engine-core` owns kernel semantics, `memory-engine-service` owns command surface, and beta boundary crates own persistence/generation/UI. | App ownership becomes cleaner, but shared service helpers would need promotion or duplication decisions first. | Hold |
| Boundary stability | Cross-client pressure is proven, but imports still depend on repo-local service/study crates rather than a separately versioned package. | Extraction now would force premature export/promote/copy choices while semantics are still under beta pressure. | Hold |

## Multi-Session Usage Receipts

Sustained value is shown by repeated session behavior, not one-off fixture parse success:

1. `crates/memory-engine-study/tests/beta_study.rs`: resume without regeneration preserves approved units, attempts, and queue state across restart.
2. `crates/memory-engine-beta-app` tests: Rust HTTP routes run source -> generate -> approve -> reveal -> submit -> restart/resume through the study boundary.
3. Rust study and beta-app tests keep reveal semantics, duplicate-submit handling, and review-state projection aligned.
4. `docs/beta/mobile-study.md` browser smoke (May 22, 2026): mobile viewport loop proved reveal/submit behavior and persisted attempt/review counts.

These receipts demonstrate recurring study-loop utility across multiple sessions and two independent client workflows.

## Minimal Contract Dependencies

If extraction is revisited, the beta app should depend on the smallest stable surface:

- Service commands only: `next-queue` and `grade/apply-review` from `createMemoryService`.
- Shared review projection DTO only: `ReviewStateProjection` and `ScheduleChange` semantics.
- Kernel semantics only: prompt grading + FSRS schedule transitions + opaque `ReviewUnitId`.

Remain app-owned (do not promote into `memory-engine-core` for extraction
readiness alone):

- `BetaPersistenceStore` snapshot/write model;
- source parsing and generation orchestration;
- draft approval choreography;
- reveal UI state, worked-solution presentation, and activity-stage copy.

## Revisit Gate (Extract Only If All Pass)

Extraction becomes ready when all conditions are true:

1. At least two independent clients plus one additional non-beta consumer require the same service DTOs unchanged.
2. Multi-session receipts continue to pass after contract changes across at least two consecutive ticket cycles.
3. The extracted app can consume only published package exports or a deliberately promoted service package surface (no deep repo-local imports).
4. Split-repo CI demonstrates parity with current in-repo proofs for queue, review apply, reveal semantics, duplicate-submit safety, and restart/resume.

Until then, holding extraction minimizes release friction while preserving fast boundary learning.
