# Memory Engine

Shared learning and memorization engine for:

- `../ruminatio`
- `../scry`
- `../caesar-in-a-year`
- `../../Documents/daybook/tools/vault-srs`

## Current Direction

`Memory Engine` should start as a **shared TypeScript-first package**, not as a deployed service.

The engine is meant to own the stable substrate that these apps keep reimplementing:

- canonical learning-domain types
- scheduling contracts and FSRS state transitions
- grading contracts
- progression graph semantics
- queue-planning primitives
- evaluation and regression fixtures

It is **not** meant to own UI, auth, billing, content taxonomy, copy, or app-specific session choreography.

## Status

Scaffolded. Slice 1 (canonical types + FSRS scheduler + deterministic
grader) is ready for `/deliver`.

- Strategic brief: [SPEC.md](./SPEC.md)
- Slice-1 context packet: [SLICE-1-KERNEL.md](./SLICE-1-KERNEL.md)
- Agent conventions: [CLAUDE.md](./CLAUDE.md), [AGENTS.md](./AGENTS.md)
- External patterns to lift: [exemplars.md](./exemplars.md)
- Backlog: `backlog.d/` (populated by `/groom`)

## Running locally

```sh
bun install
bun run ci          # typecheck + lint/format + test
dagger call ci --source=.   # same gate, in a container
```
