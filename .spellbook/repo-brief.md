# Memory Engine Repo Brief

## Vision & Purpose

`memory-engine` is a framework-free TypeScript learning kernel and modular API
workspace for learning and memorization applications. It extracts the stable
substrate of spaced repetition and answer grading into composable package
surfaces: canonical study-domain types, FSRS scheduling, deterministic grading,
progression helpers, queue primitives, rubric contracts, adapter interfaces,
fixture corpora, evals, benchmarks, and dogfood clients.

The API is the primary product surface. Experimental clients live beside the
kernel to dogfood the API, then winners may be extracted into their own apps.
Consumers and experiments keep storage, UI, session choreography, content
authoring, auth, analytics, vendor SDKs, and product-specific pedagogy outside
the kernel.

## Stack & Boundaries

- Runtime: Bun `>=1.3.0`, TypeScript `5.9`, ESM package.
- Core dependency: `ts-fsrs@5.2.3`.
- Source: `src/` is pure runtime code. No Convex, React, Hono, Node/Bun APIs,
  network clients, filesystem access, logging, or persistence in the runtime
  path.
- Testkit: `testkit/` publishes fixtures through `memory-engine/testkit`.
- Adapter surface: `src/adapters/` publishes vendor-neutral contracts and test
  doubles through `memory-engine/adapters`; real model clients stay in consumers.
- Experiments: future `experiments/` clients consume public API surfaces and the
  repo-local service prototype from outside `src/`.
- CI: `.dagger/src/index.ts` owns the containerized gate.
- Tracker: active tickets live in `backlog.d/`; closed tickets live in
  `backlog.d/_done/`.

## Load-Bearing Gate

`bun run ci` IS the gate. It shells out to `dagger call check --source=.` and
runs, in order: `bun install --frozen-lockfile`, `bun run typecheck`,
`bun run check`, `bun run coverage`, and `gitleaks dir /src --redact
--no-banner`.

`bun run ci:local` is the inner-loop subset: typecheck, Biome check, and
coverage-enforced tests. It is useful while iterating, but it is not delivery
evidence. Delivery requires a green `bun run ci`.

## Invariants

- One ticket, one branch, one PR. Branch from `master`.
- Do not improvise features. If there is no active `backlog.d/` ticket for the
  work, shape or groom first.
- TDD is the default: write the behavior test first, then implement, then
  refactor.
- Core is pure; consumers own persistence and injected time/identity.
- `ScheduleState` is the JSON-safe `ts-fsrs` card shape: snake_case fields,
  `state: 0 | 1 | 2 | 3`, and `last_review: number | null`.
- Prompt arms and grader dispatch co-evolve with `assertNever` exhaustiveness.
- `Grader.grade()` returns one `GradeResult` envelope with `rating` populated
  by the rating policy. Do not split verdict and rating into a two-call public
  protocol.
- Verdict vocabulary is fixed: `correct`, `close`, `wrong`, `revealed`.
- Do not add runtime dependencies casually. Slice 1 allowed only `ts-fsrs`; new
  runtime dependencies require shaped scope and documentation.
- No `any`, non-null assertions, or `@ts-ignore`. Biome enforces this.
- If a spec and code disagree, the spec wins until the spec is explicitly
  updated.

## Known Debts

- Slice 5 backlog now prioritizes modular API subpaths, service failure
  semantics, evals/benchmarks, and dogfood clients before visualization or
  extraction.
- `backlog.d/_done/10-scry-canary.md`,
  `backlog.d/_done/11-rubric-grading-contract.md`,
  `backlog.d/_done/12-adapter-surface.md`, and
  `backlog.d/_done/13-vault-rubric-canary.md` still have frontmatter
  `status: ready` even though they are archived under `_done/`.
- External canary branches exist and are part of the product proof:
  `/Users/phaedrus/Development/scry` has `memory-engine-canary`;
  `/Users/phaedrus/Documents/daybook/tools/vault-srs` has
  `memory-engine-rubric-canary`. Their oracles should be re-run or explicitly
  recorded before claiming consumer adoption is complete.
- The current worktree may be detached at `HEAD` in Codex worktrees. Create a
  `cx/...` branch before committing harness or code changes.

## Terminology

- Kernel: the shared pure package in this repo.
- Consumer: Ruminatio, Scry, Caesar in a Year, or Vault SRS.
- ScheduleState: persisted FSRS card state owned by the consumer.
- ReviewUnitId: opaque identity; the kernel never inspects concept-vs-phrasing
  semantics.
- Prompt: presentation and answer-evaluation data, separate from progression.
- Verdict: grading outcome vocabulary before policy maps to FSRS rating.
- Rating: FSRS rating enum exposed by the kernel.
- Testkit: exported fixture corpora consumed by kernel tests and consumer
  contract tests.
- Canary: a consumer branch proving the kernel boundary against real app tests.

## Session Signal

Recurring corrections and operator preferences:

- Do not stop at plausible implementation. Run the exact oracle named by the
  ticket and report residual unverified paths.
- Do not treat local green tests as consumer proof. Vault and Scry canaries are
  separate evidence.
- Do not widen the kernel boundary for convenience. Product-specific behavior
  belongs in consumers until a canary proves it is shared.
- Do not silently edit without a ticket when the change is feature work. Shape
  or groom first.
- Do not bypass gates or lower strictness. Fix the underlying issue.

Validated patterns:

- Atomic tickets in `backlog.d/` with explicit oracles work well here.
- Dagger owns the canonical gate; GitHub Actions just installs Dagger and calls
  `dagger call check --source=.`.
- Consumer canary branches are the right way to validate boundary pressure
  without forcing migrations.
- Fixture corpora in `memory-engine/testkit` are the right contract surface for
  consumers.
- The codebase benefits from small pure modules with simple public functions:
  `next`, `Grader.grade`, progression filters, queue selectors, and async rubric
  adapters.
- Dogfood clients should produce executable receipts under `docs/dogfood/`
  before any client or service shape is extracted.
