# Harden the deployed API: gated deploys, abuse limits, session lifecycle

Priority: P1 · Status: pending · Estimate: M

## Goal

The Fly deployment cannot ship red commits, cannot be trivially abused, and
operators have the affordances to notice and recover from failure.

## Oracle

- [x] `deploy.yml` cannot run unless the CI workflow succeeded for the same
      SHA (workflow_run wiring, exact-SHA checkout), and a post-deploy smoke
      (healthz + home + anonymous auth boundary) fails the deploy on
      regression. (2026-06-11)
- [ ] `/app/account` is rate-limited per email and per IP; test proves the
      limit.
- [x] `POST /accounts` and the authenticated save-account path enforce the
      email allowlist; non-allowlisted emails get 403, no session token.
      (2026-06-11)
- [ ] Logout route exists and revokes the server-side session; file-store
      magic-link consumption is atomic (no replay window) — postgres path
      already is.
- [x] File store already requires the explicit
      `MEMORY_ENGINE_ENABLE_FILE_STORE=true` opt-in at boot (verified
      2026-06-11; production runs Postgres on Neon, so no [mounts] needed).
- [x] `migrate()` runs once per process, not per request (2026-06-11).
      Residual: full connection pooling needs a store-level refactor
      (`PostgresStudyStore` wraps `RefCell<Client>`); per-request connects
      are acceptable against Neon's pooled endpoint meanwhile.
- [x] `docs/runbook.md` covers deploy, rollback, secrets, store backend
      selection, database (Neon), login, and Canary health. (2026-06-11)

Shipped alongside (2026-06-11, not in the original oracle): Canary error
reporting on every API 500 (`memory-engine-canary` crate, verified against
live canary-obs); production database moved to Neon (`memory-engine-prod`,
`twilight-brook-49749008`) and `OPENROUTER_API_KEY` set so prose generation
works in production. The prior Fly Managed Postgres cluster
`memory-engine-api-pg` is superseded — operator decision pending on
decommission.

## Notes

Evidence: deploy.yml has no dependency on ci.yml; no rate limit on the
mailer endpoint; no logout route; `consume_auth_challenge` file path is
read-then-write non-atomic; per-request connect+migrate acknowledged in repo
docs as pre-traffic debt. Vetted non-issue: magic-link token entropy is fine
(`rand` 0.9 ThreadRng is CSPRNG-backed) — do not spend effort there.

## Children

1. CI-gated deploy + post-deploy smoke job.
2. Rate limiting + mailer abuse tests.
3. Session lifecycle: logout, server-side revocation, expired-session sweep.
4. Store-backend boot guard + pooled postgres with startup migration.
5. Operator runbook under docs/.
