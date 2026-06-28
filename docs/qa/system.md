# Memory Engine QA System

Refs-backlog: 25
Refs-backlog: 051
Refs-backlog: 052
Refs-backlog: 053
Refs-backlog: 054
Refs-backlog: 055

## Purpose

The QA system is the repeatable proof path for `memory-engine`. It is designed
to answer two questions:

1. Does the public API still execute the learning semantics consumers depend on?
2. Where can quality improve beyond pass/fail bug finding?

The executable entrypoint is:

```sh
cargo run -p memory-engine-qa -- --local
cargo run -p memory-engine-qa -- --full
```

`cargo run -p memory-engine-qa -- --local` is the inner loop.
`cargo run -p memory-engine-qa -- --full` is the handoff path and ends with the
full `bun run ci:full` Dagger gate.

## Quality Model

QA evidence is organized around product quality, not implementation folders:

- API integrity: Rust facade exports compose without private crate imports.
- Learning semantics: scheduling, grading, progression, and queue behavior stay
  stable against fixtures and regression corpus cases.
- Contract usefulness: testkit fixtures and adapter doubles remain valid
  consumer-facing contracts.
- Boundary clarity: the Rust service boundary and dogfood clients keep persistence,
  UI, authored content, confidence, and session choreography outside the kernel.
- Drift detection: evals and benchmark receipts expose behavior and performance
  changes before clients absorb them.
- Science traceability: adopted learning-science principles remain tied to
  cited doctrine plus executable tests or benchmark receipts in
  `docs/science/README.md`.
- Handoff confidence: Rust formatting, tests, Clippy, rustdoc, Gitleaks, and
  Dagger all pass. The fast `bun run ci` gate runs host Cargo directly; the full
  `bun run ci:full` Dagger lane binds a Postgres service and sets
  `MEMORY_ENGINE_POSTGRES_TEST_URL`, so live Postgres API/store contracts run
  before handoff instead of skipping as local-only opt-in tests.

## Executable Lanes

`crates/memory-engine-qa` runs these lanes in a fixed order and prints a
receipt after each lane:

| Lane | Surface | Purpose |
|---|---|---|
| `static.rustfmt` | all Rust crates | keep checked-in Rust in canonical format |
| `static.clippy` | all Rust targets | catch correctness, maintainability, and API-shape warnings |
| `api.facade` | `memory-engine` facade crate | prove consumers can compose root, modular, testkit, and dogfood surfaces |
| `kernel.core` | `memory-engine-core` | protect pure learning semantics, queue deferral semantics, and adapter contracts |
| `service.prototype` | `memory-engine-service` command boundary | prove command flow, injected persistence, and failure semantics |
| `persistence.beta-store` | `memory-engine-persistence` durable beta store | prove persisted snapshots, restart, conflict, and validation semantics |
| `generation.beta` | `memory-engine-generation` deterministic generation probe | prove source parsing, provenance, draft validation, and promotion behavior |
| `study.beta-session` | `memory-engine-study` session/API boundary | prove source, generation, approval, reveal, answer, post-answer feedback, concept health, skip/snooze, reference, bridge, queue, and resume flow |
| `app.beta-http` | `memory-engine-beta-app` local HTTP routes | prove mobile routes and validation run through the Rust study session |
| `api.v1-contract` | versioned public JSON contract and consumer proof binary | run the Scry-facing client against a local HTTP API and prove contract fixtures stay executable |
| `dogfood.rust-receipts` | Rust CLI, import probe, web shell | exercise migrated dogfood clients through the Rust facade and service crates |
| `docs.rustdoc` | all public Rust crates | prove public API documentation compiles |
| `performance.benchmarks` | Rust facade, scheduler, queue, service, science receipts | expose migrated-runtime and learning-policy drift without brittle thresholds |
| `ci.full` | Dagger CI | prove Rust fmt, file/Postgres tests, Clippy, doc, and Gitleaks together |

All lanes are gating except `performance.benchmarks`, which is receipt-only
until the project has enough historical data to define stable budgets.

The capture-anything path adds a focused generation receipt:

```sh
cargo run -p memory-engine-bench -- generation
```

That receipt is still local and deterministic, but it is not a raw performance
benchmark: its `shape` column must stay green for the intent fixtures that
cover verbatim memorization, concept understanding, fact recall, and
procedure/process capture, its `variants` column scores same-concept same-stage
phrasing variety without answer leakage, its `dup` column uses the same
near-duplicate predicate as source generation, and its bridge fixture must stay
green for lower-stage, same-concept, non-duplicate bridge material. Live model
quality comparisons stay explicit and write dated receipts under `docs/evals/`:

```sh
cargo run -p memory-engine-bench -- generation \
  --model google/gemini-3.5-flash \
  --prompt principled \
  --judge anthropic/claude-sonnet-4.6 \
  --out docs/evals/generation-gemini-3.5-flash-judged-$(date +%F).md
```

## Operating Procedure

Use this sequence for QA work:

```sh
cargo run -p memory-engine-qa -- --local
cargo run -p memory-engine-qa -- --full
```

For a focused change, run the affected surface first, then finish with the QA
harness. Examples:

```sh
cargo test -p memory-engine-core
cargo test -p memory-engine
cargo test -p memory-engine-study
cargo test -p memory-engine-api review_escape_hatches
cargo test -p memory-engine-api post_answer_feedback
cargo test -p memory-engine-openrouter
cargo run -p memory-engine-bench -- generation
```

Report QA evidence with exact commands, final status, surfaces exercised, and
any unrun ticket-required proof oracle. Do not claim beta/product proof from
local package tests alone.

## Improvement Review

After every full QA pass, update `docs/qa/quality-register.md` when the run
reveals a quality opportunity. A register item does not need to be a bug. Good
items include missing scenario coverage, unclear API ergonomics, weak fixture
corpora, benchmark gaps, consumer-proof gaps, or dogfood friction.

Register entries should name:

- quality dimension
- current evidence
- improvement
- trigger for promoting it into a shaped backlog ticket

Do not use the register for vague wishes. If an item is actionable now and
blocks the active work, fix it instead of recording it.
