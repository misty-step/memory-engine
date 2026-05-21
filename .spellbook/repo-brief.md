# Memory Engine Repo Brief

generated: 2026-05-21T18:25:06Z

## Vision & Purpose

`memory-engine` is a framework-free TypeScript/Bun learning kernel and dogfood workspace for learning and memorization products. The package owns the stable substrate: canonical study-domain types, FSRS scheduling, deterministic and rubric grading contracts, progression helpers, queue primitives, adapter contracts, fixture corpora, evals, benchmarks, and package-surface smoke tests.

The package API is the primary product surface. Experimental clients live beside the kernel to dogfood the API and create pressure before anything is promoted or extracted. The current beta ladder is intentionally application-layer: durable beta persistence, source-grounded quiz and exercise generation, mobile study dogfood, service-contract hardening, graduated activity variants, and a later extraction decision.

## Stack & Boundaries

- Runtime: Bun `>=1.3.0`, TypeScript `5.9`, ESM package.
- Core dependency: `ts-fsrs@5.2.3`.
- `src/` is pure runtime code. No Convex, React, Hono, Node/Bun APIs, filesystem, network, logging, persistence, UI, auth, analytics, or vendor SDKs belong in the published runtime path.
- `testkit/` publishes fixture corpora through `memory-engine/testkit`.
- `src/adapters/` publishes vendor-neutral contracts through `memory-engine/adapters`; real model clients stay in consumers or experiments.
- `service/` and `experiments/` may use Bun/application-layer state to prove local workflows, but they do not move persistence or UI ownership into `src/`.
- `.dagger/src/index.ts` owns the containerized CI implementation.
- `backlog.d/` is the active tracker; `backlog.d/_done/` is the closed tracker.

## Load-Bearing Gate

`bun run ci` IS the gate. It shells out to `dagger call check --source=.` and runs install, typecheck, Biome check, coverage-enforced tests, and Gitleaks.

`bun run ci:local` is the local loop only: typecheck, Biome check, and coverage-enforced tests. `bun run qa` is the full QA sweep and ends with `bun run ci`, but QA does not replace the gate. Delivery requires a green `bun run ci` plus any ticket-named proof oracle.

## Invariants

- One ticket, one branch, one PR. Branch from `master`; use `cx/...` by default.
- Do not implement feature work without an active shaped `backlog.d/` ticket.
- TDD is the default. Write behavior tests before implementation and avoid internal mocks of repo-owned collaborators.
- Core is pure. Consumers own storage, identity, time injection, UI, sessions, parsing, content authoring, analytics, and model providers.
- `ScheduleState` is the JSON-safe `ts-fsrs` card shape: snake_case fields, `state: 0 | 1 | 2 | 3`, and `last_review: number | null`.
- `ReviewUnitId` is opaque. The kernel never infers concept-vs-phrasing meaning from it.
- Prompt union changes and grader dispatch changes ship together with `assertNever` coverage.
- `Grader.grade()` returns one `GradeResult` envelope with `rating` populated. Verdicts remain `correct`, `close`, `wrong`, `revealed`.
- No `any`, non-null assertions, or `@ts-ignore`.
- Runtime dependencies require shaped scope and docs updates.
- Do not lower gates or mark unrun canaries as proof.

## Backlog Lifecycle

Active work lives in `backlog.d/`; closed work lives in `backlog.d/_done/`. Work references use `Refs-backlog: NN`; closure uses `Closes-backlog: NN` or `Ships-backlog: NN`. Archive by sourcing `scripts/lib/backlog.sh` and using `backlog_archive`.

`/deliver` gets work merge-ready. `/settle` polishes a branch until it is green, reviewed, and coherent. `/ship` owns archive, merge, closure trailers, post-merge verification, and bounded reflection. `/groom` reconciles stale active tickets against trailers and archived files.

## Known Debts

- Slice 6 is now the main product pressure path: durable beta persistence, source-grounded AI generation, mobile study dogfood, service-contract hardening, backlog/QA hygiene, graduated activity ladders, and extraction decision work.
- Active tickets `26` and `27` appear satisfied by existing `experiments/beta-store/`, `experiments/beta-generation/`, and docs, but they still live in `backlog.d/`. `backlog.d/30-backlog-hygiene-and-qa-receipts.md` should reconcile active-vs-archived state instead of silently treating the tracker as clean.
- Several archived tickets under `backlog.d/_done/` still carry misleading `status: ready` frontmatter. The cleanup belongs in backlog hygiene, not ad hoc during unrelated feature work.
- Historical Scry and Vault canary branches are deprecated. Current proof comes from package tests, repo-local dogfood lanes, beta receipts, and explicitly shaped current external oracles.
- There is no deploy target. Monitor CI, QA, dogfood, beta, benchmark, and tracker drift; do not invent production health checks.

## Terminology

- Kernel: the pure package surfaces under `src/`.
- Consumer: an application or beta shell consuming `memory-engine`.
- ScheduleState: persisted FSRS card state owned by the consumer.
- ReviewUnitId: opaque study unit identity.
- Prompt: presentation and answer-evaluation data, separate from progression.
- Verdict: grading outcome vocabulary before policy maps to FSRS rating.
- Rating: FSRS rating enum exposed by the kernel.
- Testkit: exported fixture corpora for kernel tests and consumer contract tests.
- Dogfood lane: repo-local executable client proof, currently CLI review, import probe, web shell, beta store, and beta generation.
- External proof: explicitly shaped proof outside this package; deprecated Scry/Vault canaries are not current proof.

## Session Signal

Recurring corrections and operator preferences:

- Do not treat plausible implementation or local green tests as product proof; run the exact oracle and name unverified paths.
- Keep the beta path usable. Application-layer persistence is expected when the goal is a real study interface.
- Remove deprecated Scry/Vault proof requirements from active harness surfaces.
- Treat exercises and practice problems as first-class beta artifacts, not synonyms for quizzes.
- Do not edit feature behavior without a shaped ticket; shape or groom first.

Validated patterns:

- Atomic `backlog.d/` tickets with executable oracles are the right unit of work.
- Dagger owns the canonical gate; GitHub Actions installs Dagger and calls `dagger call check --source=.`.
- Repo-local dogfood clients and beta receipts expose boundary pressure without reviving deprecated apps.
- Fixture corpora in `memory-engine/testkit` are the consumer contract surface.
- Small pure modules with simple public functions beat service-shaped wrappers in `src/`.
- Dogfood clients should produce executable receipts under `docs/dogfood/` or `docs/beta/` before extraction or package promotion.
