# Import Probe Dogfood

Refs-backlog: 22

## Purpose

`experiments/import-probe/` converts one tiny authored fixture into canonical
`Prompt`, `QueueCandidate`, and `ScheduleState` inputs. The probe discovers
input pressure for dogfood clients without adding content parsing, taxonomy, or
AI compilation behavior to `src/`.

## Commands

```sh
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
