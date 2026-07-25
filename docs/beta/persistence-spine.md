# Beta Persistence Spine

Refs-Powder: memory-engine-026

## Purpose

`crates/memory-engine-persistence` is the durable persistence proof for Scry's
beta interface. It is intentionally repo-local and outside the pure kernel:
the beta product needs saved state now, but the published kernel should remain
pure until repeated clients prove a stable service contract.

## Ownership Boundary

Belongs to the beta store:

- source documents and source permissions;
- reference spans used to justify generated prompts;
- generated learning drafts, including prompt or exercise drafts, critique
  notes, validation status, and model metadata;
- generation run receipts;
- learner-kept review units and their queue metadata;
- learner attempts and schedule records;
- applied-review receipts used for duplicate and stale-write protection.

Belongs to the kernel/API:

- canonical `Prompt`, `QueueCandidate`, `ScheduleState`, grading, scheduling,
  progression, and queue semantics;
- pure functions in `crates/memory-engine-core` and facade exports from
  `crates/memory-engine`;
- fixture/eval contracts that prove shared learning behavior.

No filesystem, database, provider SDK, network, or UI code is added under
`crates/memory-engine-core`.

## Storage Shape

The current implementation uses one JSON snapshot file with atomic full-file
replacement. This is deliberately small: it proves restart/reload and write
safety without introducing a database dependency before the beta workflow earns
one.

Snapshot collections:

- `sourceDocuments`: typed text/link/file/image/video-transcript metadata,
  body or URI, permission label, freshness, and creation time.
- `referenceSpans`: cited source ranges or excerpts linked to a source
  document.
- `generatedPromptDrafts`: current storage name for beta-generated learning
  drafts. Today these are canonical prompt drafts; future beta work may add
  exercise drafts with worked solutions, activity kind, ladder stage, scoring
  rubric, and validation status before anything enters review.
- `generationRuns`: provider/model run receipts and validation failures.
- `reviewUnits`: kept prompt, prompt id, reference links, queue metadata,
  and generated-draft linkage.
- `schedules`: JSON-safe `ScheduleState` records keyed by `ReviewUnitId`.
- `attempts`: `ServiceAttemptRecord` history.
- `appliedReviews`: idempotency/natural-key receipts for applied reviews.

## Atomicity And Retry Contract

The store prepares the full next snapshot in memory, writes it to a temporary
file, then renames it into place. The in-memory snapshot advances only after
the file replacement succeeds.

`grade/apply-review` safety is enforced at the store boundary:

- the service passes the exact prior schedule it read;
- the store compares that prior schedule with the current durable schedule;
- stale writes reject with `StaleScheduleWriteError`;
- duplicate applied reviews reject with `DuplicateAppliedReviewError`;
- injected write failures leave the previous snapshot reloadable with no
  partial attempt or schedule mutation.

This keeps duplicate submits and retries from silently advancing FSRS state
twice. Future beta work may add explicit client idempotency keys and a richer
transport error envelope, but ticket 26 proves the minimum safety boundary.

## Queue Reload Assumptions

`listQueueCandidates()` hydrates queue candidates from approved review units
plus the latest persisted schedule records. It intentionally keeps the
candidate shape compatible with the existing queue primitive:

- top-level `due` remains the queue ordering contract;
- stored schedules replace candidate schedule snapshots on reload;
- progression metadata, prerequisites, supersession, concept/source/domain
  keys, and anti-clumping metadata survive restart.

The beta store does not yet implement pagination, indexes, or partial due
queries. Those belong in a later ticket if the mobile beta produces enough
content to make full-pile loading painful.

## Privacy Assumptions

The beta store can hold private learner content. Committed tests use synthetic
fixtures only. Source documents carry a permission label:

- `local-only`: do not send to hosted model providers;
- `model-eligible`: may be used by a product-level generation workflow.

The pure kernel does not own transport policy, but the current beta generation
runner, study reference path, bridge path, forwarding fallback, and external
provider adapters enforce this label before any model transmission. Permission
updates are scoped to the owning account and active (non-archived) source.

## Extraction Criteria

Do not promote this store as a public package surface yet. Revisit extraction
only after the mobile beta proves repeated real use and at least one of these is
true:

- multiple clients need the same durable store contract;
- the beta needs a real database with migration and indexing semantics;
- service DTOs, idempotency keys, queue hydration, and generated-content
  provenance stabilize across more than one workflow.
- graduated activity metadata, quiz/exercise drafts, and worked-solution
  records prove reusable across multiple beta workflows without importing
  provider, UI, or persistence concepts into `crates/memory-engine-core`.

Until then, `crates/memory-engine-persistence` is a dogfood spine, not Scry's
database.

## Verification

Ticket 26 evidence:

```sh
cargo test -p memory-engine-persistence
bun run ci
```

The focused test suite covers restart/reload, applied-review duplicate safety,
failed-write atomicity, realistic queue-pile reload, generated-draft validation,
and promotion from accepted draft to review unit.

The former TypeScript `experiments/beta-store/` runtime oracle was deleted
after the Rust crate covered durable store behavior and wire-shape parity.
