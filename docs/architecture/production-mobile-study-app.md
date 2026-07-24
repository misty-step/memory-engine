# Context Packet: Production Mobile Study App

> Historical context packet (2026-06-06). Its provider choice and deployment
> commands were superseded by the 2026-07-08 DigitalOcean cutover. Preserve it
> as shaping evidence; use `docs/runbook.md` for current operations.

## PRD Summary

- User: motivated learner using a phone to turn trusted source material into
  short review sessions.
- Problem: the Rust beta app proves the study loop locally, but it has no
  account boundary, production persistence, provider-backed generation, or
  deployment proof.
- Why now: the Rust cutover is complete and extraction is explicitly on hold
  until production-facing boundary evidence exists.
- UX enabled: paste source material, keep cited generated prompts, save an
  account-backed study set, and return to scheduled reviews.
- Deliverable type: working code plus deployment-ready production boundary.
- Success signal: a staging-deployed Rust API serves an account-scoped
  source-to-review smoke path without leaking auth, persistence, provider, or
  UI concerns into `memory-engine-core`.

## Goal

Ship a simple mobile-optimized study application boundary that supports account
creation, source-backed study material generation, and scheduled review on
production infrastructure.

## Product Requirements

- P0: A learner can create or save an account, submit source material, generate
  study drafts, keep or edit useful material, review due material, and resume
  from persisted state.
- P0: Reveal remains display-only and never mutates attempts, reps, or schedule
  state.
- P0: Account/user identity scopes all mutable production state.
- P0: Generated material must carry source evidence and rejection reasons when
  source material is unsupported or unsafe.
- P0: Production deployment uses a long-running Rust service with managed
  durable storage, not a serverless route scatter.
- P1: Mobile UI defers account creation until the first useful study set exists
  unless the user explicitly chooses to sign in first.
- P1: Deployment receipts include staging URL, health check, one review
  round-trip, and persistence-after-restart proof.
- Non-goals: billing, native app stores, multi-region active-active writes,
  general tutoring chat, lesson narration, and extraction into a separate repo.

## Non-Goals

- Do not add auth, SQL, model clients, filesystem, network, logging, or UI
  dependencies to `crates/memory-engine-core`.
- Do not promote beta UI metadata into kernel semantics to satisfy one
  production shell.
- Do not claim product proof from `bun run ci` alone.
- Do not use Fly volumes or the current JSON file store as the production
  source of truth for account-backed state.

## Constraints / Invariants

- `bun run ci` remains the fast repo gate; `bun run ci:full` remains the
  Dagger-backed handoff gate.
- `memory-engine-core` stays framework-free and persistence-free.
- `memory-engine-service` remains the command boundary for queue selection and
  grade/apply-review.
- Auth, account scoping, provider clients, deployment, HTTP, UI, and production
  storage are boundary concerns.
- `ReviewUnitId` remains opaque.
- Verdicts remain `correct`, `close`, `wrong`, and `revealed`.
- Every production review write requires an idempotency key and optimistic
  concurrency proof.

## Authority Order

tests > type system > code > docs > memory/lore

## Repo Anchors

- `crates/memory-engine-core/src/lib.rs` — pure learning kernel that must stay
  free of production infrastructure.
- `crates/memory-engine-service/src/lib.rs` — typed command boundary to reuse.
- `crates/memory-engine-persistence/src/lib.rs` — current file-backed beta
  store and contract reference, not the production database adapter.
- `crates/memory-engine-generation/src/lib.rs` — deterministic generation
  boundary to preserve while adding provider adapters.
- `crates/memory-engine-study/src/lib.rs` — source/generate/keep/reveal/
  submit choreography to reuse as product pressure.
- `crates/memory-engine-beta-app/src/lib.rs` — mobile server-rendered dogfood
  shell and route tests.
- `docs/beta/service-contract-v0.md` — local service contract and reveal
  policy.
- `docs/qa/system.md` — QA lane and gate contract.

## Prior Art

- `docs/beta/mobile-study.md` — phone-sized local smoke flow and UX friction.
- `docs/beta/extract-beta-app-readiness.md` — current decision to hold
  extraction.
- `docs/beta/persistence-spine.md` — durable beta-store semantics to preserve
  through a production adapter.
- `docs/beta/content-generation.md` — source-backed generation and validation
  pressure.
- `exemplars.md` — learning-semantics precedents; do not copy app-specific
  session builders into the kernel.

## Product Forms Considered

| Option | Shape | Strength | Failure Mode | Verdict |
|---|---|---|---|---|
| Phone-first web app | Mobile web UI over Rust API | Fastest to deploy, test, and iterate | Can become dashboard-heavy if beta panels leak into first viewport | Choose |
| Native mobile app | iOS/Android client over Rust API | Best eventual mobile feel | App-store and client-build overhead before backend proof | Reject for first slice |
| Local-first Tailscale app | Personal server with phone smoke | Strong dogfood continuity | Not account-backed production infrastructure | Defer |
| API-only service | Rust API without UI | Deep backend proof | Does not satisfy mobile app UX | Reject alone |
| Vercel full stack | Serverless functions plus frontend | Excellent static/front-end deployment | Shallow route scatter; weak fit for Rust service and scheduled state | Reject |
| Managed no-code auth/app shell | Outsource UI/auth heavily | Fast first account screen | Weak agent-readiness and opaque source of truth | Reject |

## Alternatives Considered

| Option | Shape | Strength | Failure Mode | Verdict |
|---|---|---|---|---|
| Fly Machines + managed Postgres | Long-running Rust service on Fly, Postgres as source of truth | Best Rust/process fit, good mobile latency, scriptable deploy | More ops ownership than Railway; Fly volumes must not become database | Choose |
| Railway + Railway Postgres | Rust service and database on Railway | Low setup friction, simple project canvas | Less explicit runtime control; can freeze weak boundaries because deploy is easy | Fallback |
| Vercel Rust Functions | Rust serverless functions | Strong edge/network and frontend story | Each route becomes function-shaped; bad fit for one deep service boundary | Reject for backend |
| Render + managed Postgres | Docker service with boring managed database | Conservative, simple operational model | Single-region default and less mobile latency control | Fallback |
| VPS + managed Postgres | Hand-managed Rust service | Maximum control, portable Docker | High ops/security/backups burden for early product | Reject now |
| Fly + volume JSON store | Deploy current beta app with mounted `/data` | Fast hosted dogfood proof | Single-writer demo only; not production account persistence | Defer as smoke-only |
| Supabase/Neon DB + API anywhere | Managed Postgres plus portable Rust API | Clear data boundary and migration story | Still needs auth/API/deploy discipline | Accept as database shape |

## Tradeoff Matrix

| Option | Fit | Size | Privacy | Agent-manageable | Reversible | Testable | Operating Burden |
|---|---:|---:|---:|---:|---:|---:|---:|
| Fly Machines + managed Postgres | 5 | 3 | 4 | 4 | 4 | 5 | 3 |
| Railway + Railway Postgres | 4 | 4 | 4 | 4 | 4 | 4 | 4 |
| Vercel Rust Functions | 2 | 3 | 3 | 2 | 3 | 3 | 4 |
| Render + managed Postgres | 4 | 4 | 4 | 4 | 5 | 4 | 4 |
| VPS + managed Postgres | 3 | 2 | 4 | 2 | 5 | 4 | 1 |
| Fly + volume JSON store | 3 | 5 | 2 | 5 | 3 | 3 | 4 |

Fly with managed Postgres scores highest because it preserves a long-running
Rust service boundary while giving a credible production persistence path.
Railway and Render remain credible fallbacks. Vercel is rejected for the
backend because it biases the design toward many shallow functions. Fly plus
the existing JSON store is useful only as a dogfood smoke lane because it has
single-writer and no account-isolation guarantees.

## Technical Design

- Chosen architecture: Rust `memory-engine-api` boundary service deployed to
  Fly Machines, backed by managed Postgres and an external auth provider or
  narrow app-owned auth adapter.
- Files/systems touched:
  - new `crates/memory-engine-api` for HTTP/auth/session boundary;
  - new `crates/memory-engine-persistence-postgres` for account-scoped
    production storage behind service contracts;
  - `crates/memory-engine-generation` for provider adapter boundary while
    preserving deterministic fixture mode;
  - `crates/memory-engine-qa` for deployed smoke receipts;
  - root `Cargo.toml`, Dagger CI, Dockerfile, and Fly config;
  - docs under `docs/architecture`, `docs/beta`, and `docs/qa`.
- Data/control flow:
  1. User reaches mobile web app.
  2. Source is submitted through account-scoped HTTP route.
  3. Generation adapter stores a generation receipt plus cited draft rows.
  4. User keeps/skips/edits drafts.
  5. Kept drafts become review units.
  6. Review uses `memory-engine-service` for next queue and grade/apply-review.
  7. Postgres adapter enforces user scope, idempotency, and compare-and-apply.
- Current implementation state:
  - `crates/memory-engine-persistence-postgres` now owns the first production
    schema and account-scoped store boundary for source documents, reference
    spans, generation runs, generated drafts, review units, schedules,
    attempts, durable applied-review receipts, service-store queue reads, and
    beta-store snapshot reconstruction. `crates/memory-engine-study` now uses a
    generic storage contract, and its source/generate/keep/reveal/submit flow
    passes against an account-scoped Postgres store in the opt-in live contract
    test.
  - `crates/memory-engine-api` now selects the Postgres account store at runtime
    when `MEMORY_ENGINE_POSTGRES_URL` is present, with
    `MEMORY_ENGINE_API_STORE_DIR` retained only as an explicitly opted-in local
    file-backed fallback. A live API test drives the JSON
    source/generate/keep/reveal/submit routes through an isolated Postgres
    schema and verifies source persistence after API state recreation.
  - `fly.toml` no longer configures a mounted JSON volume. Staging and
    production must provide `MEMORY_ENGINE_POSTGRES_URL` as a secret.
- Build/check boundary:
  - focused tests for API routes, auth isolation, Postgres store contracts, and
    generation adapter receipts;
  - `cargo run -p memory-engine-qa -- --local`;
  - `bun run ci:full`;
  - staging deploy smoke against `/healthz`, source/generate/keep/review, and
    restart/resume.
- ADR decision: required. Production shell, account boundary, and database
  choice are durable architecture decisions.
- Design X vs Y: choose a cohesive Rust API over serverless functions; choose
  managed Postgres over Fly volume JSON for production state; defer native app
  until the mobile web flow proves repeated value.

## Agent Readiness

- Profile source: missing repo-local `.harness-kit/agent-readiness.yaml`;
  global roster available at `/Users/phaedrus/.harness-kit/agents.yaml`.
- Stack feedback strength: Rust compiler, clippy, rustdoc, Dagger, and
  behavior-focused tests are strong; browser/deploy evidence must be added.
- ADR decision: required for production shell and account/data boundary.
- Infrastructure path: CLI/API-managed Fly deploy plus managed Postgres; avoid
  dashboard-only state.
- Gate: `bun run ci` and `bun run ci:full`, with staging deploy smoke once
  deployment config exists.
- Evidence storage: `docs/qa/`, `docs/beta/`, and future `.tmp/qa/` or
  non-source receipt path for screenshots/reports.
- Mock policy impact: preserved if tests use real repo-owned service/store
  integration and mock only external auth/provider/network boundaries.

## Delegation Evidence

- Roster providers used: Codex CLI deployment critic, Claude CLI architecture
  critic, Pi CLI agent-readiness critic.
- Native subagents used: repo investigator, product/design critic, architecture
  critic, test/oracle reviewer.
- Accepted evidence:
  - production path needs account-scoped persistence before real deployment
    claims;
  - Fly Machines fit a long-running Rust service better than Vercel Functions;
  - account creation should be deferred until a useful generated study set
    exists in the first-run UX;
  - reveal must remain display-only and duplicate submit must be idempotent;
  - Postgres-backed adapter is the production path, while file-backed JSON is
    dogfood-only.
- Rejected evidence:
  - current beta app plus Fly volume is not enough for account-backed
    production;
  - native mobile is premature before backend proof;
  - Vercel backend is rejected for this service boundary, though a future
    marketing/static front end could use it.
- Waivers:
  - native role-specific subagents using unavailable fixed models failed and
    were replaced with default native lanes;
  - no provider made code edits during shaping.

## Premise Source

Premise Source Waiver: the premise is the operator-provided active thread goal
requesting `/design`, `/shape`, `/deliver`, production deployment evaluation,
subagent fanout, agent-readiness optimization, Rust, modular design, and
automated QA. The request exists in the agent thread rather than a repo-local
artifact.

Residual risk: future implementers cannot verify the original operator wording
from this repo alone without the session transcript.

## Exemplar Techniques

- Deterministic grading parity from
  `/Users/phaedrus/Documents/daybook/tools/vault-srs/src/grading.ts` — preserve
  grading semantics while adding production shells.
- FSRS wrapper compactness from
  `/Users/phaedrus/Development/caesar-in-a-year/lib/srs/fsrs.ts` — keep the
  scheduler surface small.
- Concept-vs-phrasing boundary from
  `/Users/phaedrus/Development/scry/convex/fsrs/engine.ts` — keep
  `ReviewUnitId` opaque.

## Oracle (Definition of Done)

- [x] Powder card `memory-engine-040` records the production-app acceptance
  oracle.
- [x] ADR records Fly Machines + managed Postgres as the chosen production
  shell and rejects Vercel as the backend.
- [x] A Rust API boundary exists outside `memory-engine-core`.
- [x] Account/session routes scope all mutable state to an account/user.
- [x] Production persistence adapter passes the same service-store behavior
  required by file-backed beta persistence against live Postgres.
- [x] Generation adapter records provider/model receipts and preserves
  deterministic fixture mode for tests.
- [x] Mobile smoke covers source capture, generation, approval, account save,
  review, reveal, submit, next, restart/resume, and no horizontal overflow.
- [x] Staging deploy proof includes `/healthz` and one account-scoped
  source-to-review round trip.
- [x] `cargo run -p memory-engine-qa -- --local`, `bun run ci`, and
  `bun run ci:full` pass.

## Deliverable

- Output: production mobile study app boundary, deploy config, QA evidence, and
  updated docs.
- Acceptance oracle: context packet plus Powder card plus executable tests,
  staging deployment smoke, and full CI.
- Evidence artifacts: test output, Dagger output, staging URL, screenshots,
  `/state` or API receipts, and deploy command output.
- Residual risk: production auth provider, Postgres provider, and model vendor
  choice may need separate credential/config setup outside this repo.

## Observability Plan

- Changed behavior to watch: account creation/session start, source ingestion,
  generation acceptance/rejection, review submission, duplicate submit,
  schedule changes, and restart/resume.
- Named signal or evidence surface: request logs, generation receipts,
  applied-review receipts, health endpoint, QA report, and deploy smoke
  receipt.
- Instrumentation debt if no signal exists: add structured request and
  generation/review event receipts before calling the deployment production
  ready.

## Acceptance Evidence

- Acceptance source: this context packet, `SPEC.md`,
  `docs/beta/extract-beta-app-readiness.md`, `docs/beta/mobile-study.md`,
  `docs/qa/quality-register.md`, and the future Powder card.
- Evidence that proves it: implementation tests, browser/mobile screenshots,
  deployed smoke, and Dagger output.
- Exact command/path/route exercised:
  - `cargo test -p memory-engine-api -p memory-engine-persistence-postgres`;
  - `MEMORY_ENGINE_POSTGRES_TEST_URL=postgres://test:test@127.0.0.1:5432/sploot_test cargo test -p memory-engine-persistence-postgres live_postgres_store_scopes_accounts_and_persists_idempotent_reviews -- --nocapture`;
  - `cargo test -p memory-engine-generation -p memory-engine-study -p memory-engine-beta-app -p memory-engine-persistence`;
  - `cargo run -p memory-engine-qa -- --local`;
  - `bun run ci:full`;
  - `flyctl deploy -a memory-engine-api --remote-only`;
  - `curl -fsS https://memory-engine-api.fly.dev/healthz`;
  - deployed JSON account-scoped source-to-review routes;
  - `flyctl machine restart 84e474a4266518 -a memory-engine-api`;
  - `flyctl machine restart 080395df316758 -a memory-engine-api`;
  - deployed 390 x 844 Chromium mobile smoke.
- Oracle / acceptance artifact hash:
  - `sha256:17bbad7e8fe284c81d7fb9d02688e701a7f42f89fa89ecb2e8c277dc8c2569c3 SPEC.md`;
  - `sha256:720019fa8725c29539312d77ffb7b1e9647ad926ba54b0285cd67954432ed558 docs/beta/extract-beta-app-readiness.md`;
  - `sha256:50905ff028920ee638ce0495b3a62f24af8edb9063d9a677ca4ed3dd25ae7de2 docs/beta/mobile-study.md`;
  - `sha256:4945f34fbfbbeeddaf77913c5b03ef56763af00868942939ed2bb6cb6ac373d4 docs/qa/quality-register.md`.
- Contract-change acknowledgment: this packet changes no executable contract;
  implementation must record any acceptance-contract change.
- Residual risk: external auth provider and model provider credentials remain
  future integrations; Fly Managed Postgres is provisioned and attached.
