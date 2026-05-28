# Mobile Beta Study Interface

Refs-backlog: 28

## Purpose

`crates/memory-engine-study` and `crates/memory-engine-beta-app` are the local
beta interface over the persistence spine and deterministic generation probe.
They are intentionally application-layer code: source input, approval state,
reveal state, worked-solution display, and mobile UI behavior stay outside the
published kernel.

## Executable Receipt

Focused oracle:

```sh
cargo test -p memory-engine-study -p memory-engine-beta-app
```

Covered behaviors:

- source material creation;
- quiz and exercise draft generation;
- accepted draft approval into review units;
- queue selection through the service boundary;
- answer submission for quiz prompts and worked-solution entry for exercises;
- reveal display with expected answer and worked solution;
- grade/apply-review with persisted schedule change;
- next-item projection;
- restart/resume from persisted state without regenerating content;
- duplicate submit protection after a graded answer.

Local browser target:

```sh
BETA_STUDY_STORE=.tmp/beta-study/store.json bun run rust:beta-study
```

The Rust shell serves `http://127.0.0.1:4174`, persists a JSON beta store, and
uses the existing phone-friendly HTML layout. The former TypeScript
`experiments/beta-study/` runtime oracle was deleted after the Rust crates
covered session, persistence, and HTTP route parity. The HTML asset remains
shared by the Rust host.

Browser smoke receipt on May 22, 2026:

- viewport override: 390 x 844;
- loaded `http://127.0.0.1:4174`;
- saved the bundled NATO source fixture;
- generated one quiz draft and one exercise draft;
- approved both drafts;
- revealed and submitted the exercise answer `CHARLIE ALFA TANGO`;
- repeated reveal and submit after grading to verify the UI still showed one
  attempt;
- verified `scrollWidth === clientWidth` before and after interaction;
- verified the visible result showed expected answer, worked solution, correct
  grade, one attempt, and one review rep.

## UX Friction

- The first screen can show source entry, draft approval, or review depending on
  persisted state. That is useful, but the app still needs stronger empty-state
  recovery around malformed source blocks.
- Exercise solving works through the same grading path as quiz review. That is
  acceptable for deterministic beta fixtures, but richer exercises will need a
  clearer rubric/result display before provider-backed generation.
- Reveal remains UI-owned. The service has no reveal command and should not gain
  one until repeated beta evidence says reveal has scheduling consequences.
- Review state is compactly projected for the interface, but the projection is
  still assembled in beta-study code rather than a stable DTO.

## API Pressure

- `BetaPersistenceStore` is carrying the right durable shape for source,
  generation, approval, attempts, schedules, and applied-review receipts.
- `memory-engine-service` is sufficient for `next-queue` and
  `grade/apply-review`; no package export or runtime persistence was needed.
- Worked solutions and activity kind/stage remain beta metadata. They should not
  move into `src/` until the graduated ladder ticket proves provider-neutral
  semantics across more than one fixture.
- Duplicate submit behavior is best handled at the interface boundary for now:
  once a local item is graded, a repeated submit returns the existing view
  instead of invoking another review.

## Boundary Verdict

The database/service boundary still feels right. The kernel owns grading,
scheduling, queue metadata, and service commands. The beta app owns source
authoring, deterministic generation, approval, reveal display, worked solutions,
session projection, and mobile UI state.
