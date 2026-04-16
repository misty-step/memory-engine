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

This directory currently contains the shaping/specification packet only.

- Main brief: [SPEC.md](./SPEC.md)
- Recommendation: library-first, service-later only if the evidence earns it
- Scope: shared learning semantics, not a new workflow or product shell
