# Scry

Scry is the quiz-first consumer product: **Remember everything** across a
phone-first PWA, CLI, skill, MCP, and API over one capability system. `VISION.md`
is the product contract.

## Product and runtime

- All faces share typed capabilities and learning/grading semantics.
- Beta access is invite-gated with a visible waitlist and magic-link sign-in;
  there is no OAuth path. Machine faces use operator-gated service sessions.
  Public signup waits for bounded cost, privacy, reliability, and Stripe proof.
- Production is the native Rust `memory-engine-api` process on Misty Step's
  isolated DigitalOcean public application host, backed by Neon Postgres at
  `https://scry.study`.
- Crate names, Postgres identifiers, wire and telemetry literals, and
  `MEMORY_ENGINE_*` environment variables retain the old name. Renaming a
  storage, network, deployment, or compatibility boundary requires a shaped
  issue and migration proof.

## Architecture and boundaries

- This is a Rust workspace; `Cargo.toml` owns the crate graph.
- `crates/memory-engine-core` is framework-free and persistence-free. It has no
  Convex, React, Hono, Node/Bun APIs, filesystem, network, logging, auth,
  analytics, UI state, vendor SDK, or model-client dependency. Boundary crates
  own storage, ingestion, sessions, UI, identity, analytics, and model clients.
- `.dagger/src/index.ts` is the only retained TypeScript surface. Browser JS
  under `crates/memory-engine-api/assets/` (service worker and page bootstrap)
  is the deliberate plain-JS exception; it has no build step and is covered by
  Rust route/render tests plus the `app.js` contract test.

## Sources of truth

- `VISION.md` governs product promise, quiz loop, five faces, product bars,
  Rust boundaries, and production surface.
- `SPEC.md` and `docs/rust-migration.md` provide strategy and cutover history;
  use `VISION.md` when positioning conflicts.
- `SLICE-*.md` and `exemplars.md` are historical extraction context, not
  delivery oracles.
- GitHub Issues is authoritative for shaped work; work from the operator's
  current request. Check current code and overlapping work; record ownership
  and verification evidence in the session or PR. Use `Refs #<issue>` in commits
  and pull requests; use `Closes #<issue>` only when all acceptance criteria are
  satisfied and verified.
- `.dagger/src/index.ts` owns CI behavior. `docs/qa/system.md`,
  `docs/dogfood/`, and `docs/beta/` hold executable QA and dogfood evidence.
  `docs/runbook.md` is the production runtime and smoke contract.
- Resolve conflicts in this order: tests, type system, code, docs, lore.

## Runtime contracts

- The scheduler receives `ScheduleState` and returns the next state; consumers
  own persistence. `ScheduleState` is JSON-safe snake_case with
  `state: 0 | 1 | 2 | 3` and `last_review: number | null`. `ReviewUnitId` is
  opaque; the kernel does not infer concept or phrasing meaning.
- Prompt enum and grader dispatch changes require exhaustive Rust matches and
  grader tests in the same change. `Grader::grade()` returns one `GradeResult`
  with `rating` populated by the injected rating policy.
- Verdicts are `correct`, `close`, `wrong`, or `revealed`; other names map to
  these four and need a spec update.
- Do not add runtime dependencies without shaped scope and docs. Do not lower
  gates, bypass hooks, or claim unrun canaries as proof.
- No TypeScript runtime or tests belong outside `.dagger/`; the QA crate enforces
  this by extension, while the browser-JS exception above remains permitted.

## Gates and proof

- `bun run ci` is the fast host gate: format, workspace tests, Clippy, and
  rustdoc. `bun run ci:full` is the Dagger-backed ship-parity gate with
  containerized Postgres, the pinned Rust image, and Gitleaks.
- `bun run ci:local` and `bun run rust:ci` are fast-gate aliases. `bun run qa`
  is the full QA sweep and ends with `bun run ci:full`; it does not replace the
  fast gate. Use the ticket's named proof oracle and the current Scry surface
  for live-product decisions.
- Test observable behavior with real repo-owned collaborators; mock only
  external boundaries such as network, clock, and model providers.

## Layout

- `memory-engine-core`: kernel; `memory-engine`: facade and testkit.
- `memory-engine-service`, `-persistence`, `-generation`, `-openrouter`, and
  `-study`: service, local persistence, source-backed generation, model HTTP,
  and study boundaries. Only `-openrouter` talks to a model network.
- `memory-engine-api` and `-api-state`: production routes, static assets,
  auth/session state, storage adapters, and generation jobs.
- `memory-engine-api-render`, `-beta-app`, and `-web-shell`: rendered UI and
  local dogfood hosts. `-cli`, `-import`, `-bench`, and `-qa` provide clients,
  import, benchmark, and QA receipts.
- `.dagger/` is owned CI code and receives normal review.

## Current pressure

Keep the Rust cutover complete and operator docs pointed at Rust, Cargo, Dagger,
and the production runbook. Prefer repeated phone-sized Rust dogfood receipts
over speculative client architecture; keep boundary complexity out of the pure
kernel. General-purpose hosting/auth frameworks, chat tutoring, and generalized
content import are outside the product.
