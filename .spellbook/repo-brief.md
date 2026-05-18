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

- Slice 6 now prioritizes a usable beta interface: durable beta persistence
  outside `src`, AI content-generation provenance, mobile-first study dogfood,
  service-contract hardening, backlog/QA hygiene, and a later beta extraction
  decision.
- `backlog.d/_done/18-modular-api-surface.md`,
  `backlog.d/_done/19-service-boundary-failure-semantics.md`,
  `backlog.d/_done/20-evals-and-benchmarks-baseline.md`,
  `backlog.d/_done/21-cli-review-loop-dogfood.md`, and
  `backlog.d/_done/22-content-normalization-probe.md` still have frontmatter
  `status: ready` even though they are archived under `_done/`.
- `backlog.d/16-system-visualization-workbench.md` remains useful but later;
  it should not displace beta usefulness work unless architecture confusion
  starts causing repeated defects.
- Historical Scry and Vault canary branches are deprecated and are not part of
  current harness proof. Current product proof comes from repo-local dogfood
  lanes, beta interface receipts, and any explicitly shaped non-deprecated
  external oracle.
- The current worktree may be detached at `HEAD` in Codex worktrees. Create a
  `cx/...` branch before committing harness or code changes.

## Terminology

- Kernel: the shared pure package in this repo.
- Consumer: an application or beta shell consuming the memory-engine package.
- ScheduleState: persisted FSRS card state owned by the consumer.
- ReviewUnitId: opaque identity; the kernel never inspects concept-vs-phrasing
  semantics.
- Prompt: presentation and answer-evaluation data, separate from progression.
- Verdict: grading outcome vocabulary before policy maps to FSRS rating.
- Rating: FSRS rating enum exposed by the kernel.
- Testkit: exported fixture corpora consumed by kernel tests and consumer
  contract tests.
- External proof: a current, explicitly shaped application or beta-shell oracle
  proving the kernel boundary against real usage.

## Session Signal

Recurring corrections and operator preferences:

- Do not stop at plausible implementation. Run the exact oracle named by the
  ticket and report residual unverified paths.
- Do not treat local green tests as product proof. Repo-local dogfood lanes and
  shaped beta receipts are the current proof path.
- Do not widen the kernel boundary for convenience. Product-specific behavior
  belongs in the beta/application layer until repeated dogfood or shaped
  external proof shows it is shared.
- Do not silently edit without a ticket when the change is feature work. Shape
  or groom first.
- Do not bypass gates or lower strictness. Fix the underlying issue.

Validated patterns:

- Atomic tickets in `backlog.d/` with explicit oracles work well here.
- Dagger owns the canonical gate; GitHub Actions just installs Dagger and calls
  `dagger call check --source=.`.
- Repo-local dogfood clients and explicit beta receipts are the default way to
  validate boundary pressure without reviving deprecated apps.
- Fixture corpora in `memory-engine/testkit` are the right contract surface for
  consumers.
- The codebase benefits from small pure modules with simple public functions:
  `next`, `Grader.grade`, progression filters, queue selectors, and async rubric
  adapters.
- Dogfood clients should produce executable receipts under `docs/dogfood/`
  before any client or service shape is extracted.
