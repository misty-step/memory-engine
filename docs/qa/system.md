# Memory Engine QA System

Refs-backlog: 25

## Purpose

The QA system is the repeatable proof path for `memory-engine`. It is designed
to answer two questions:

1. Does the public API still execute the learning semantics consumers depend on?
2. Where can quality improve beyond pass/fail bug finding?

The executable entrypoint is:

```sh
bun run qa:local
bun run qa
```

`bun run qa:local` is the inner loop. `bun run qa` is the handoff path and ends
with the canonical `bun run ci` Dagger gate.

## Quality Model

QA evidence is organized around product quality, not implementation folders:

- API integrity: public exports compose without private `src` imports.
- Learning semantics: scheduling, grading, progression, and queue behavior stay
  stable against fixtures and regression corpus cases.
- Contract usefulness: testkit fixtures and adapter doubles remain valid
  consumer-facing contracts.
- Boundary clarity: the service prototype and dogfood clients keep persistence,
  UI, authored content, confidence, and session choreography outside the kernel.
- Drift detection: evals and benchmark receipts expose behavior and performance
  changes before clients absorb them.
- Handoff confidence: typecheck, Biome, coverage, Gitleaks, and Dagger all pass.

## Executable Lanes

`crates/memory-engine-qa` runs these lanes in a fixed order and prints a
receipt after each lane:

| Lane | Surface | Purpose |
|---|---|---|
| `static.typecheck` | all package code included by `tsconfig` | catch type drift across public contracts and behavior tests |
| `static.biome` | repo source, tests, scripts, docs-adjacent code | catch formatting, unused imports, and unsafe patterns |
| `api.exports` | root and modular package exports | prove consumers can compose the API without private imports |
| `kernel.types-scheduler` | types and scheduler | protect `ScheduleState` shape and FSRS round-trip semantics |
| `kernel.grader` | deterministic and rubric grading | protect one-envelope grade results and fixed verdict semantics |
| `kernel.progression-queue` | progression and queue | prove actual learning-flow selection behavior |
| `contracts.testkit-adapters` | `testkit` and `adapters` | prove shared fixtures and adapter doubles remain usable |
| `service.prototype` | repo-local service boundary | prove command flow, injected persistence, and failure semantics |
| `evals.regression-corpus` | fixtures through live API surfaces | catch semantic drift across core behaviors |
| `dogfood.rust-receipts` | Rust CLI, import probe, web shell | exercise migrated dogfood clients through the Rust facade and service crates |
| `dogfood.ts-oracles` | TypeScript CLI, import probe, web shell | keep legacy dogfood behavior executable until TypeScript deletion |
| `coverage.all` | all Bun tests | preserve broad coverage evidence |
| `performance.benchmarks` | Rust facade, scheduler, queue, service | expose migrated-runtime performance drift without brittle thresholds |
| `ci.canonical` | Dagger CI | prove install, typecheck, Biome, coverage, and Gitleaks together |

All lanes are gating except `performance.benchmarks`, which is receipt-only
until the project has enough historical data to define stable budgets.

## Operating Procedure

Use this sequence for QA work:

```sh
bun run qa:local
bun run qa
```

For a focused change, run the affected surface first, then finish with the QA
harness. Examples:

```sh
bun test tests/grader/
bun test tests/queue/
bun test tests/api/module-exports.test.ts tests/api/compatibility.test.ts
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
