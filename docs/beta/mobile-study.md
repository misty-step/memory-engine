# Mobile Beta Study Interface

Refs-backlog: 28

## Purpose

`experiments/beta-study/` is the local phone-first beta interface over the
persistence spine and deterministic generation seam. It is intentionally
application-layer code: source input, approval state, reveal state,
worked-solution display, and mobile UI behavior stay outside the published
`src/` kernel.

## Executable Receipt

Focused oracle:

```sh
bun test experiments/beta-study/
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
- one-input arbitrary-prose generation into source-backed drafts;
- one-at-a-time keep, skip, and edit decisions before study starts;
- measured browser response time instead of a hardcoded answer latency.

Local browser target:

```sh
BETA_STUDY_STORE=.tmp/beta-study/store.json bun run experiments/beta-study/server.ts
```

The shell serves `http://127.0.0.1:4174`, persists a JSON beta store, and uses a
phone-friendly single-column layout below 760px.

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

Browser smoke receipt on May 31, 2026:

- viewport override: 390 x 844;
- loaded `http://127.0.0.1:4176`;
- first viewport showed one visible input surface and one primary action:
  `Make practice`;
- pasted arbitrary prose: `Alpha is the first Greek letter. Beta is the second
  Greek letter. Gamma measures the rate of change of Delta.`;
- generated cited drafts and showed one approval card at a time with source
  excerpt, question, expected answer, and Keep / Skip / Edit controls;
- kept the first draft, verified `Start practice` appeared while the next draft
  stayed in approval;
- started practice, revealed the expected answer, verified reveal left attempts
  at `0`, submitted an answer, and verified attempts became `1`;
- forced a duplicate submit after grading and verified attempts remained `1`;
- advanced to the next card;
- verified `scrollWidth === clientWidth` at every step and captured
  `.tmp/beta-study-037/mobile-smoke.png`.

## UX Friction

- The first screen now starts from one source input when the store is empty, then
  morphs into approval or review based on persisted state. Malformed source
  recovery is clearer, but the failure copy is still heuristic and should be
  revisited after live-provider receipts.
- Exercise solving works through the same grading path as quiz review. That is
  acceptable for deterministic beta fixtures, but richer exercises will need a
  clearer rubric/result display before provider-backed generation.
- Reveal remains UI-owned. The service has no reveal command and should not gain
  one until repeated beta evidence says reveal has scheduling consequences.
- Review state is compactly projected for the interface and hidden behind
  details on the main phone path. The projection is still assembled in
  beta-study code rather than a stable DTO.

## API Pressure

- `BetaPersistenceStore` is carrying the right durable shape for source,
  generation, approval, attempts, schedules, and applied-review receipts.
- `createMemoryService` is sufficient for `next-queue` and
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
