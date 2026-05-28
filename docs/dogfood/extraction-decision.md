# Extraction Decision

Refs-backlog: 24

## Decision

Keep experimenting in this repository. Do not extract a standalone application
repository yet, and do not promote the Rust service boundary as a public package
surface yet.

The current CLI review, import probe, and web shell prove that the public API
surfaces are usable from outside `src/`. They also show real pressure around
learner-facing review-state DTOs, reveal semantics, session choreography,
persistence, and generated-content provenance. That pressure is not stable
enough to freeze into a public service contract or separate product repository.

There are two separate gates:

- beta usefulness requires one durable interface that survives restart/resume,
  duplicate submits, failed writes, realistic queue reloads, and repeated manual
  dogfood sessions;
- service or app extraction requires repeated pressure across clients or
  workflows after the durable beta exists.

## Evidence Reviewed

| Experiment | Evidence | Decision pressure |
| --- | --- | --- |
| CLI review | `crates/memory-engine-cli` runs a calibration-aware review loop through the service boundary. | Attempts, confidence, grading, scheduling, and queueing compose cleanly, but receipt formatting and confidence policy stay product-owned. |
| Import probe | `crates/memory-engine-import` compiles authored material into canonical prompts, queue candidates, and schedule state. | Canonical API types can represent imported learning inputs; parsers and authoring policy should remain outside the kernel. |
| Web shell | `crates/memory-engine-web-shell` renders a local study loop and exercises answer, reveal, review-state visibility, and queue transitions. | A usable interface needs review-state presentation, reveal policy, persistence, and eventually content generation. One web client is not enough proof to stabilize these as package contracts. |
| QA harness | `bun run qa` runs public exports, kernel behavior, the Rust service boundary, eval corpus, dogfood clients, benchmarks, and Dagger CI. | Package confidence is strong; product readiness still needs beta persistence, content generation, and mobile dogfood receipts. |

## What Stays In `memory-engine`

- Pure learning-kernel runtime under `src/`.
- Public subpaths for types, scheduling, grading, progression, queue, adapters,
  and testkit fixtures.
- Rust service boundary and repo-local dogfood experiments outside `src/`.
- Dogfood evidence, evals, benchmarks, and quality registers.

## What The Beta Interface May Own

The beta interface needs persistence immediately. That does not mean the
published kernel owns a database. The beta application can live in this repo as
an experiment while owning:

- local database or file-backed storage for sources, generated prompts, review
  units, attempts, schedules, references, and generation runs;
- content ingestion for typed text, pasted documents, files, images, links, and
  later video transcripts;
- AI generation, critique, de-duplication, and provenance workflows;
- mobile-first review UI, answer entry, reveal, feedback, repair, and queue
  presentation;
- product policy for privacy, source permissions, and model-provider calls.

Stable, provider-neutral contracts can move back toward the kernel only after
durable beta evidence shows the behavior is real and repeated clients or
workflows need the same abstraction.

## Follow-Up Tickets

- `26-beta-persistence-spine`: create the repo-local beta data model and
  durable local persistence boundary outside `src/`, including restart/reload,
  realistic queue, and idempotent review-write proof.
- `27-ai-content-generation-probe`: generate quiz drafts from persisted source
  material with provenance and validation receipts.
- `28-mobile-beta-study-interface`: make a mobile-first beta shell usable for
  real dogfood review sessions.
- `29-service-contract-v0-hardening`: decide DTOs, reveal semantics, typed
  failures, and public-subpath-only service tests.
- `30-backlog-hygiene-and-qa-receipts`: reduce tracker drift and persist QA
  receipts when useful.
- `31-beta-extraction-decision`: revisit extraction, helper promotion, and
  database ownership after durable beta evidence exists.

## Revisit Criteria

Revisit beta usefulness when one interface proves durable source ingestion,
generation approval, review, restart/resume, retry safety, realistic queueing,
and repeated manual dogfood. Revisit extraction when repeated clients or
workflows independently need the same service commands, DTOs, reveal semantics,
and persistence-independent contracts. Revisit database ownership only if
multiple consumers need a shared hosted memory service rather than client-owned
storage with a shared kernel.
