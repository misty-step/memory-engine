---
shaping: true
ticket: 17-service-scenario-fixtures
slice: 4
status: shipped
priority: high
estimate: M
depends_on: [15-service-interface-prototype]
oracles:
  - bun run ci
  - bun test tests/service/session-flow-fixtures.test.ts
  - test -f docs/service-scenarios.md
---

# Service scenario fixtures - prove the first review loop

## Goal

Add representative, versioned service-level fixtures that prove the prototype
can run a narrow learning loop from authored prompt data through attempt
recording, grading, schedule application, and next-queue selection without
moving persistence, session choreography, or content parsing into `src/`.

## Non-Goals

- No production HTTP, RPC, CLI, worker, or daemon adapter.
- No extraction to a separate repository.
- No durable storage implementation.
- No framework/runtime imports in `src/`.
- No general content authoring parser.
- No product-specific session builder, tutor prompt, streak, XP, or analytics
  policy.

## Oracle

- [x] `tests/service/session-flow-fixtures.test.ts` defines at least two
      representative scenario fixtures: one deterministic prompt loop and one
      progression-aware queue loop.
- [x] The fixture runner executes `record-attempt`, `grade/apply-review`, and
      `next-queue` against an in-memory `MemoryServiceStore` fake that validates
      the same command and persistence contracts the prototype exposes.
- [x] The fixtures assert emitted attempts, persisted `ScheduleState`, and queue
      choice from canonical expected outputs rather than snapshotting incidental
      object shapes.
- [x] `docs/service-scenarios.md` records what the scenarios prove, what remains
      app-owned, and which service/application extraction questions are still
      unanswered.
- [x] No published `memory-engine` export is added for the service prototype
      unless the implementation notes justify that the surface is stable enough
      to become package API.
- [x] `bun run ci` exits 0.

## Notes

Ticket 15 pinned the first command envelope and persistence boundary. The next
highest-leverage question is whether those commands compose into a useful review
loop before we invest in visualization or extraction planning.

Keep the fixtures concrete. Prefer a small authored-object fixture translated
by the test into existing `Prompt`, `QueueCandidate`, and `ScheduleState` data
over a reusable parser or workflow DSL. The test should reveal service-boundary
pressure, not hide it behind harness code.

If the scenario needs behavior not expressible through the current service
commands, shape the missing command explicitly instead of adding product flags.

## Study

### Problem Diamond

User outcome: prove that the dedicated memory-service direction can support a
real review loop, not just isolated command calls.

Boundary pressure: authored material and session advancement are tempting to
centralize, but the current repo contract says the kernel owns semantic
primitives while consumers own parsing, persistence, and product choreography.

Falsifying canary: a scenario fixture should fail if the service command
surface cannot connect grading, scheduling, and queue selection without
inspecting product-specific identity or content taxonomy.

### Alternatives

Minimal service-fixture approach: keep all new work outside `src/`, add a
contract-style scenario runner under `tests/service/`, and document the first
loop evidence. This is the selected path because it tests composition while
preserving the kernel boundary.

Stricter consumer-owned approach: leave scenario fixtures to the future extracted
app and only keep unit-level command tests here. This keeps `memory-engine`
smaller, but it delays the most important extraction question: whether the
service form factor actually supports a coherent learning loop.

Broader prototype approach: build a local CLI or web shell now. This would
produce interface feedback sooner, but it introduces runtime and UX choices
before the command lifecycle has enough executable evidence.
