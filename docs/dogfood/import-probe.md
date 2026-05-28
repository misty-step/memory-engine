# Import Probe Dogfood

Refs-backlog: 22

## Purpose

`crates/memory-engine-import` converts one tiny authored fixture into canonical
`Prompt`, `QueueCandidate`, and `ScheduleState` inputs, then runs the first
compiled prompt through the Rust service loop. The probe discovers input
pressure for dogfood clients without adding content parsing, taxonomy, or AI
compilation behavior to the reusable kernel.

The TypeScript `experiments/import-probe/` path remains a migration oracle until
the TypeScript runtime is deleted.

## Commands

```sh
bun run rust:import-probe
cargo test -p memory-engine-import
bun test experiments/import-probe/import-probe.test.ts
bun run experiments:import-probe
```

## Authored Fixture

Fixture name: `latin-prayer-authored-v1`

The authored material contains two source phrases from the Latin Mass ordinary:

- `Credo in unum Deum`
- `Pater noster`

The probe compiles them into short-answer prompts, queue candidates, and one
existing review schedule.

## Essential API Fields

- `reviewUnitId`
- prompt kind and prompt text
- accepted answers
- equivalence groups
- ignored punctuation tokens
- queue due timestamp
- concept, source, and domain keys
- JSON-safe schedule state for already-reviewed material

## Product-Owned Fields

- `sourceText`
- `translation`
- `confidencePrompt`
- `notes`

## API Gap

No kernel API gap surfaced. The current public types can represent the compiled
learning inputs, while authored content, confidence copy, notes, and compiler
policy remain client-owned.

## Boundary Notes

This is an import probe, not a parser framework. It uses a canned authored
fixture and a deterministic compiler so future AI-assisted compilers can be
evaluated against the same canonical output contract before any product client
depends on them.

The Rust crate validates that authored cards still carry product-owned study
metadata, but only canonical prompts, queue candidates, prompt IDs, and schedule
state cross into the service boundary.
