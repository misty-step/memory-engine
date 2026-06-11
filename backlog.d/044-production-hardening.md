# Harden the deployed API: gated deploys, abuse limits, session lifecycle

Priority: P1 · Status: pending · Estimate: M

## Goal

The Fly deployment cannot ship red commits, cannot be trivially abused, and
operators have the affordances to notice and recover from failure.

## Oracle

- [ ] `deploy.yml` cannot run unless the CI workflow succeeded for the same
      SHA (workflow_run or required-check wiring), and a post-deploy smoke
      (healthz + auth + study round-trip) fails the deploy on regression.
- [ ] `/app/account` is rate-limited per email and per IP; test proves the
      limit.
- [ ] `POST /accounts` no longer issues a session token without email
      verification or allowlist enforcement (found live during ticket-42 QA:
      open registration on production; see docs/qa/real-clock-receipt.md).
- [ ] Logout route exists and revokes the server-side session; file-store
      magic-link consumption is atomic (no replay window) — postgres path
      already is.
- [ ] API binary refuses to boot in production with the file store unless
      explicitly opted in (prevents silent data loss on machine recycle —
      fly.toml has no [mounts]).
- [ ] Postgres connections are pooled or reused and `migrate()` runs once at
      startup, not per request (`memory-engine-api/src/lib.rs:2370,2393`).
- [ ] A runbook in docs/ covers: deploy, rollback, secrets, store backend
      selection, and how to check production health.

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
