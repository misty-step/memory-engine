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

`crates/memory-engine` is now the consumer-facing Rust facade:

- root exports mirror the current TypeScript package ergonomics for grading,
  scheduling, progression, queueing, and canonical types;
- the `adapters` namespace mirrors the TypeScript `memory-engine/adapters`
  rubric adapter surface with a vendor-neutral trait and static test double;
- modular namespaces preserve deep ownership instead of asking consumers to
  depend on every internal crate directly;
- `testkit` exposes Rust fixture corpora for grading, recitation, scheduling,
  progression, and queue contract tests without moving fixture construction
  into the pure kernel;
- beta and dogfood modules expose repo-local application surfaces without
  promoting those concerns into the pure kernel;
- facade tests prove README-style root usage, modular package-path composition,
  Rust testkit fixture replay, and dogfood receipt access through the Rust
  surface.

`crates/memory-engine-core` ports the first pure-kernel surface:

- domain types for prompts, grades, schedule state, progression metadata, and
  queue candidates;
- deterministic grading for MCQ, boolean, cloze, short answer, and recitation;
- rubric prompt, rubric assessment, async-grader facade, adapter trait, static
  adapter double, confidence normalization, criterion normalization, and
  no-adapter failure semantics without adding any model client or async runtime
  to the pure kernel;
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
- shared service command scenarios in `fixtures/service-command-scenarios.json`
  now execute against both the TypeScript service oracle and the Rust service
  crate, covering record-attempt, grade/apply-review, next-queue, and
  progression unlock behavior.

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

The former TypeScript beta-study session and server were deleted after the Rust
study and beta-app crates covered session, persistence, and HTTP route parity.
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

`crates/memory-engine-web-shell` now ports the local web-shell dogfood path:

- builds a learner-facing session view over the Rust import-probe fixture;
- keeps reveal status, compact review-state DTOs, prompt copy, answer draft
  flow, and interface-pressure receipt formatting in the shell boundary;
- drives `next-queue` and `grade/apply-review` through `memory-engine-service`;
- serves the existing web-shell HTML and `/state`, `/reveal`, `/answer`, and
  `/next` routes from a Rust binary.
- emits the web-shell extraction receipt with `bun run rust:web-shell:receipt`
  so dogfood QA no longer needs to launch the TypeScript shell for receipts.

The operator-facing `experiments:cli-review`, `experiments:import-probe`, and
`experiments:web-shell` scripts now resolve to Rust implementations. Their
TypeScript predecessors were deleted after the Rust crates covered receipt,
service-loop, and HTTP route parity.

`crates/memory-engine-bench` now ports the benchmark receipt path:

- runs grading, scheduler advancement, queue selection, and service
  grade/apply-review plus next-queue loops through Rust APIs;
- keeps benchmark receipts non-gating and threshold-free until historical data
  justifies stable budgets;
- makes `bun run bench` a Rust runtime command instead of a TypeScript package
  benchmark.

`crates/memory-engine-qa` now ports the QA receipt runner:

- preserves the local/full lane model, exact command receipts, gating versus
  non-gating behavior, and canonical Dagger handoff lane;
- keeps the remaining TypeScript oracle tests as explicit executed lanes while
  moving the orchestration runtime to Rust;
- makes `bun run qa:local` and `bun run qa` Rust runtime commands.

`crates/memory-engine-coverage` now ports the coverage floor gate:

- keeps Bun as the TypeScript oracle test runner with `bun test --coverage`;
- moves summary parsing and floor enforcement into Rust;
- makes `bun run coverage` a Rust runtime command while preserving the same
  80% function and line floors.

## Parity Strategy

The Rust tests intentionally mirror current Bun behavior first. Broader parity
requires:

- shared JSON fixtures for grading, progression, queue, and scheduler cases;
- deeper JSON fixture coverage beyond the first scheduler new/learning/review
  and relearning transitions;
- broader service failure fixtures that execute both TypeScript and Rust command
  envelopes until cutover, including storage conflict and malformed-attempt
  failures;
- beta-study browser smoke tests against the Rust server before TypeScript
  deletion.

## Cutover Matrix

| Surface | Current owner | Rust status | Cutover evidence |
| --- | --- | --- | --- |
| Package facade | TypeScript `package.json` exports | Rust `memory-engine` facade crate | Root and modular facade tests |
| Testkit fixtures | TypeScript `testkit/` | Rust facade `testkit` module | Rust fixture replay through public surfaces |
| Domain types | TypeScript `src/types.ts` | First core port | JSON fixture parity |
| Deterministic grading | TypeScript `src/grader.ts` | First core port | Fixture parity and property tests |
| Rubric grading and adapters | TypeScript `src/async-grader.ts`, `src/adapters/` | Rust core rubric boundary plus facade `adapters` namespace | Core rubric parity tests and facade export test |
| Progression | TypeScript `src/progression.ts` | First core port | Vault/Ruminatio-style fixtures |
| Queue | TypeScript `src/queue.ts` | First core port | Priority and anti-clump fixtures |
| Scheduling | TypeScript `src/scheduler.ts` | Rust core port | Shared JSON fixture parity |
| Service | TypeScript `service/` | Rust command crate with shared scenario parity | Shared command fixtures plus failure fixture parity |
| Persistence | TypeScript `experiments/beta-store/` | First Rust crate port | Store commit/restart tests |
| Beta generation | Deleted TypeScript `experiments/beta-generation/` | Rust generation crate | Deterministic generation fixture parity |
| Beta study app | Deleted TypeScript runtime under `experiments/beta-study/`; HTML asset retained | Rust session/API and HTTP host port | Phone/browser smoke on Rust host |
| CLI dogfood | Deleted TypeScript `experiments/cli-review/` | Rust CLI port | Receipt parity |
| Import probe | Deleted TypeScript `experiments/import-probe/` | Rust import crate port | Authored fixture and service-loop receipt parity |
| Web shell | Deleted TypeScript runtime under `experiments/web-shell/`; HTML asset retained | Rust web-shell crate port | Session, receipt, and HTTP route parity |
| Bench receipts | TypeScript `scripts/bench.ts` | Rust benchmark crate | Non-gating `bun run bench` receipt |
| QA receipts | TypeScript `scripts/qa.ts` | Rust QA crate | Local/full lane tests plus `bun run qa` |
| Coverage gate | TypeScript `scripts/check-coverage.ts` | Rust coverage crate | Coverage parser tests plus `bun run coverage` |
