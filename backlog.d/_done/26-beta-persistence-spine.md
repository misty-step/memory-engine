---
shaping: true
ticket: 26-beta-persistence-spine
slice: 6
status: shipped
priority: high
estimate: M
depends_on: [24-extraction-decision-gate]
oracles:
  - bun run ci
  - bun test experiments/beta-store/
  - test -f docs/beta/persistence-spine.md
---

# Beta persistence spine - local app state outside the kernel

## Goal

Create the repo-local beta persistence boundary needed for a usable memory
engine interface without moving database ownership into the published `src/`
kernel.

The beta interface needs saved source material, generated prompts, review
units, schedules, attempts, references, and generation receipts. Those records
belong in the beta application layer until repeated clients prove a stable
shared service contract.

## Non-Goals

- No database, filesystem, network, auth, or model-provider code under `src/`.
- No hosted service.
- No public package export for beta persistence.
- No migration framework beyond the minimum needed for local beta restart tests.
- No private user data fixture committed to the repo.

## Oracle

- [ ] `experiments/beta-store/` defines a durable local persistence boundary
      outside `src/` with stores for source documents, reference spans,
      generated prompt drafts, review units, attempts, schedules, queue
      candidates, and generation runs.
- [ ] Tests prove a review session can persist an attempt, update schedule
      state, preserve references, and reload the queue from storage.
- [ ] Restart/reload tests prove source material, generated drafts, approved
      prompts, attempts, schedule state, references, and queue candidates
      survive process/session recreation.
- [ ] `grade/apply-review` persistence is atomic or rejects cleanly under
      failed writes, retries, and duplicate submits using compare-and-apply
      semantics or idempotency keys.
- [ ] Queue reload tests exercise a nontrivial uneven pile with due reviews,
      fresh items, progression groups, prerequisites, supersession, and source
      anti-clumping.
- [ ] Tests prove generated prompt drafts carry source ids, model/provider
      metadata, validation status, and provenance before becoming review units.
- [ ] `docs/beta/persistence-spine.md` documents the schema, ownership boundary,
      privacy assumptions, idempotency/atomicity contract, queue indexing
      assumptions, and what would be required before extraction.
- [ ] `bun run ci` exits 0.

## Notes

- Bun's local runtime APIs are acceptable in `experiments/`, but the kernel
  boundary remains framework-free and persistence-free.
- Use the smallest durable store that proves restart/reload behavior. A
  file-backed or SQLite store under `experiments/` is acceptable; a purely
  in-memory fixture is not sufficient for this ticket.
- This ticket is the bridge between "pure kernel" and "actually usable beta":
  the database exists, but it is owned by the beta shell, not the package core.

## Closure Evidence

- Implemented in `experiments/beta-store/` with durable beta-owned persistence
  and no runtime persistence under `src/`.
- Documented in `docs/beta/persistence-spine.md`.
- Verified during backlog hygiene with `bun test experiments/beta-store/
  experiments/beta-generation/`.
