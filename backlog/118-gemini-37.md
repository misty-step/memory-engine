---
id: 118
status: proof
priority: p0
type: maintenance
---

# Use Gemini 3.7 Flash for generation

## Outcome

Every production generation path uses `google/gemini-3.7-flash`.

## Why now

Operator ruling during 2026-08-17 dogfood.

## Acceptance

- [x] Repository default is `google/gemini-3.7-flash`.
- [x] Host `MEMORY_ENGINE_GENERATION_MODEL=google/gemini-3.7-flash`.
- [ ] A live capture or bridge receipt names that model.

## Dependencies

None.

## Proof

Merged as `391fb55` (#128). Host env updated on deploy.
Live job receipt remains.

## Non-goals

No bakeoff. No prompt rewrite.
