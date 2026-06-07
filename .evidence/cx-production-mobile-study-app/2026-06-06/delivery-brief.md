# Delivery Brief: Production Mobile Study App

Backlog `040-production-mobile-study-app-boundary` now has a production Rust
service boundary for a phone-first study app. The branch adds `memory-engine-api`
for HTTP/account/mobile flows, `memory-engine-persistence-postgres` for
account-scoped durable state, generic store seams in generation/study so the
pure kernel stays clean, and Fly/Docker deployment config backed by Fly Managed
Postgres.

The user-visible behavior is now real on deployed infrastructure: a learner can
paste source text, generate cited drafts, save account email, keep a draft,
reveal, submit a review, and continue to scheduled review. The deployed Fly app
also survived Machine restarts and cross-Machine routing after the API session
state was moved into Postgres. A follow-up deployed smoke also proved that
duplicate review submit with the same client idempotency key remains idempotent
after API state recreation with the original session token.

## Roster Lanes

- Codex CLI was used as a deployment/delivery critic. Accepted: secret handling,
  deployed route proof, and multi-Machine persistence proof needed to be
  explicit. Rejected: none from the accepted pass.
- Claude CLI was attempted as an architecture critic but failed because of the
  provider monthly spend limit.
- Pi CLI was used as a production/agent-readiness critic. Accepted: staging
  proof, mobile proof, and ADR acceptance status had to be resolved. Rejected:
  stale unmanaged Postgres command guidance because current Fly CLI uses
  managed Postgres commands.
- Agy was redispatched for final repo-fit critique; any blocking findings from
  that pass must be resolved before the branch is called fully merge-ready.

The lanes were parallel critic lanes, not competing implementation attempts,
because the implementation was already present and the remaining task was
closeout validation.

## Why This Design

Fly Machines plus managed Postgres was chosen over Vercel Functions, Railway,
Render, VPS, and Fly volume JSON because it keeps one cohesive long-running
Rust service boundary while giving account-backed durable storage and
scriptable deployment. Vercel remains a possible future static frontend host,
but it is a poor backend fit for this scheduled-review service. Fly volume JSON
was rejected as production storage because it does not prove account-backed
durability.

The design improves agent readiness by keeping core learning semantics pure,
placing production concerns in boundary crates, and making the proof surfaces
explicit: Dagger CI, behavior tests, deployed route smoke, mobile render smoke,
artifact hashes, and a repo-local evidence packet.

## Completion Gate

- Exact end-user behavior changed: source-first mobile study flow with account
  save/resume, generated drafts, review reveal/submit/next, and durable
  restart/resume on deployed infrastructure.
- Live repo evidence read: `AGENTS.md`, backlog `040`, ADR, context packet,
  Fly QA receipt, Cargo workspace, new API/Postgres crates, generation/study
  store seams, git status/diff.
- Acceptance source: `backlog.d/040-production-mobile-study-app-boundary.md`
  plus `docs/architecture/production-mobile-study-app.md`.
- Evidence packet dir: `.evidence/cx-production-mobile-study-app/2026-06-06`.
- Evidence that proves it: `demo.md`, `docs/qa/fly-staging.md`, deployed
  health/source-to-review smoke, restart/resume proof, mobile 390 x 844 smoke,
  and `bun run ci`.
- Exact command/path/route exercised: see `evidence-index.md` for the command
  list and route list.
- Oracle / acceptance artifact hash: recorded in `evidence-index.md` and
  `receipt.json`.
- Contract-change acknowledgment: no acceptance criteria were weakened; the
  production-only multi-Machine failure strengthened the test oracle.
- Agent readiness delta: improved.
- Repo-fit check: production HTTP/database/deploy concerns remain outside
  `memory-engine-core`; tests use real repo-owned service/store integration.
- Hardening run / waiver: critic hardening found a durable idempotency gap after
  restart; fixed by threading the client key into the service attempt and
  regression-tested against Postgres.
- Formal-spec ladder evidence: not required.
- Learning packet: `learning-packet.md`.
- Reflect checkpoint evidence: not required.
- Residual risk: external auth, live model provider adapter, hidden form session
  tokens in the current no-JavaScript shell, Postgres pooling/telemetry, and
  ongoing Fly resource cost.
