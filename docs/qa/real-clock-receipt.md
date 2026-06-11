# Real-Clock Cutover Receipt (ticket 42)

Date: 2026-06-10 · Deployed commit: `5cad6a6` · Surface:
`https://memory-engine-api.fly.dev` (production, Postgres store)

## Claim

The served binary schedules reviews and expires auth artifacts on wall-clock
time. A correctly answered review must not be immediately due again.

## Evidence

JSON-route loop against production, account `acct_d901fa0479c7ab34`:

1. `POST /accounts` → 201 with session token.
2. `POST /accounts/{id}/sources` (NATO structured source) → 201
   `src_3512ab1c2a3b7f6e`.
3. `POST .../sources/{sourceId}/generate` → draft
   `study-run-1-draft-src-3512ab1c2a3b7f6e-1-nato-letter-a`.
4. `POST .../drafts/{draftId}/approve` → 200.
5. `GET .../review/next` → current =
   `generated-quiz-src-3512ab1c2a3b7f6e-1-nato-letter-a` (due).
6. `POST .../review/{unit}/submit` answer `ALFA` →
   `attemptCount: 1, lastOutcome: correct`.
7. `GET .../review/next` → `current: null` — the answered unit is scheduled
   into the real future, not immediately due.

Local gates on the shipped HEAD: `bun run ci` (Dagger) exit 0; api crate
tests include clock-injected regressions for auth-challenge TTL,
browser-session expiry (cache hit and storage reload), and multi-day
review scheduling.

## Residual

- `POST /accounts` issues a session token with no email verification or
  allowlist check — open registration on production. Tracked under ticket
  044 (production hardening).
- Postgres-path expiry behavior is asserted by inspection and unit tests
  against the file store; postgres-in-CI is ticket 045.
