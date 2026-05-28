# Rust Migration

Refs-backlog: 38

## Target Shape

The migration target is a Rust library/application stack with the same learning
semantics as the current TypeScript package and beta-study experiments. The
TypeScript code remains the executable oracle until Rust has parity coverage and
the app surfaces have moved.

## Design Rules

- Keep the Rust core pure: no filesystem, network, logging, auth, UI, model
  clients, or service framework code.
- Hide concepts behind deep modules. Callers should ask for grading,
  progression filtering, queue selection, scheduling, service commands, or
  persistence commits; they should not assemble those algorithms from shallow
  helper wrappers.
- Keep `ReviewUnitId` opaque. Concept, phrasing, and activity identity are
  caller-owned mappings.
- Keep mastery policy injected because the source apps intentionally disagree.
- Keep service and storage as later crates with typed command/result enums.

## Current Rust Slice

`crates/memory-engine-core` ports the first pure-kernel surface:

- domain types for prompts, grades, schedule state, progression metadata, and
  queue candidates;
- deterministic grading for MCQ, boolean, cloze, short answer, and recitation;
- progression eligibility with strict and fallback modes;
- queue due filtering, priority ordering, anti-clumping, and progression
  fallback;
- scheduler advancement through a Rust `Scheduler` trait and default
  `FsrsScheduler`, pinned to the current TypeScript FSRS-6 fixture outputs.

`crates/memory-engine-service` now ports the first command boundary:

- closed `MemoryServiceCommand` and `MemoryServiceResult` enums for
  record-attempt, grade/apply-review, and next-queue workflows;
- `MemoryServiceStore` as the persistence trait, including the
  `expected_prior_schedule_state` seat needed for later optimistic
  concurrency;
- `MemoryService` orchestration that keeps read schedule -> grade -> schedule
  advance -> apply review as one deep operation instead of asking callers to
  reassemble kernel helpers;
- serde `kind` tags that match the TypeScript service envelope names for later
  cross-runtime fixture replay.

`crates/memory-engine-persistence` now ports the first beta-store boundary:

- file-backed `BetaPersistenceStore` for source documents, reference spans,
  generation runs, generated drafts, review units, schedules, attempts, and
  applied-review receipts;
- typed `BetaStoreError` variants for duplicate reviews, stale schedule writes,
  missing references, missing generation runs, and injected commit failures;
- atomic temp-file-to-rename commits with copy-on-write snapshots so failed
  commits do not corrupt persisted history;
- a concrete `MemoryServiceStore` implementation for the Rust service crate;
- camelCase beta-store envelope fields for the durable snapshot while the
  `ScheduleState` contract remains the existing JSON-safe scheduler shape.

`crates/memory-engine-generation` now ports the deterministic beta-generation
probe:

- source-block parsing for concept/question/answer/reference fields;
- deterministic IDs, slugs, duplicate signatures, stage ordering, and
  validation reasons matching the TypeScript beta oracle;
- reference span, generated draft, and generation-run writes through the Rust
  beta store;
- accepted/rejected draft tests for quiz, exercise, duplicate, unsupported,
  missing-provenance, and bad-source paths;
- core serde tests for TypeScript-compatible prompt tags, numeric ratings,
  numeric schedule states, camelCase app fields, and preserved snake_case
  `ScheduleState` internals.

`crates/memory-engine-study` now ports the beta-study session/API boundary:

- source intake, generation invocation, draft approval, reveal state,
  grade/apply-review, and queue advancement as one session object;
- API DTOs for sources, drafts, queue rows, current review state, schedule
  changes, summary, and API-pressure notes with TypeScript-compatible serde
  field names;
- duplicate-submit-after-grade behavior that is view-only instead of issuing a
  second store write;
- persisted resume behavior through the Rust beta store, with no regeneration
  required;
- tests that mirror the current mobile beta session flow and a JSON wire-shape
  test for the mobile API contract.

`crates/memory-engine-beta-app` now ports the local beta-study HTTP host:

- serves the existing mobile `index.html` from a Rust binary;
- exposes the existing `/state`, `/source`, `/generate`, `/approve`, `/reveal`,
  `/answer`, and `/next` routes over the Rust `memory-engine-study` session;
- validates malformed JSON payloads before touching session state;
- keeps HTTP parsing/status-code mapping in the app host, not in the kernel,
  service, persistence, generation, or study crates.

The web UI markup is still shared from `experiments/beta-study/index.html`.

`crates/memory-engine-cli` now ports the CLI review dogfood path:

- runs the Latin-prayer review fixture through the Rust service boundary;
- keeps fixture content, confidence capture, calibration metrics, receipt
  formatting, and the in-memory dogfood store outside the reusable kernel;
- emits a JSON receipt from `bun run rust:cli-review`;
- tests the receipt and the CLI store validation boundary.

`crates/memory-engine-import` now ports the authored-content import dogfood path:

- compiles the Latin-prayer authored fixture into canonical prompts, queue
  candidates, prompt IDs, and schedule state;
- validates that source text, translations, confidence copy, and notes remain
  product-owned import metadata rather than reusable-kernel concerns;
- runs the first imported review through the Rust service boundary and selects
  the next queue item;
- emits a JSON receipt from `bun run rust:import-probe`.

## Parity Strategy

The Rust tests intentionally mirror current Bun behavior first. Broader parity
requires:

- shared JSON fixtures for grading, progression, queue, and scheduler cases;
- deeper JSON fixture coverage beyond the first scheduler new/learning/review
  and relearning transitions;
- service scenario fixtures that execute both TypeScript and Rust command
  envelopes until cutover, including storage conflict and malformed-attempt
  failures;
- beta-study browser smoke tests against the Rust server before TypeScript
  deletion.

## Cutover Matrix

| Surface | Current owner | Rust status | Cutover evidence |
| --- | --- | --- | --- |
| Domain types | TypeScript `src/types.ts` | First core port | JSON fixture parity |
| Deterministic grading | TypeScript `src/grader.ts` | First core port | Fixture parity and property tests |
| Progression | TypeScript `src/progression.ts` | First core port | Vault/Ruminatio-style fixtures |
| Queue | TypeScript `src/queue.ts` | First core port | Priority and anti-clump fixtures |
| Scheduling | TypeScript `src/scheduler.ts` | Rust core port | Shared JSON fixture parity |
| Service | TypeScript `service/` | First Rust crate port | Command scenario parity |
| Persistence | TypeScript `experiments/beta-store/` | First Rust crate port | Store commit/restart tests |
| Beta generation | TypeScript `experiments/beta-generation/` | First Rust crate port | Deterministic generation fixture parity |
| Beta study app | TypeScript `experiments/beta-study/` | Rust session/API and HTTP host port | Phone/browser smoke on Rust host |
| CLI dogfood | TypeScript `experiments/cli-review/` | Rust CLI port | Receipt parity |
| Import probe | TypeScript `experiments/import-probe/` | Rust import crate port | Authored fixture and service-loop receipt parity |
