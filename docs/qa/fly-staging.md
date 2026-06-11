# Fly Staging Scaffold

## Scope

This scaffold is for an operator-only staging smoke of `memory-engine-api`.
The runtime is now wired for managed Postgres through `MEMORY_ENGINE_POSTGRES_URL`;
the file store remains a local fallback only when explicitly opted in.

## Config

- `Dockerfile` builds `memory-engine-api`.
- `fly.toml` exposes HTTP on internal port `8080`.
- `HOST=0.0.0.0` is required so Fly Proxy can reach the service.
- `min_machines_running = 1` keeps one Machine warm for production smoke and
  avoids documenting a zero-capacity production default.
- `MEMORY_ENGINE_POSTGRES_URL` must be set as a Fly secret for staging or
  production storage, for example:

```sh
fly secrets set MEMORY_ENGINE_POSTGRES_URL=postgres://...
```

- `MEMORY_ENGINE_API_STORE_DIR=/path/to/accounts` is accepted only as a local
  file-backed fallback when `MEMORY_ENGINE_POSTGRES_URL` is absent and
  `MEMORY_ENGINE_ENABLE_FILE_STORE=true`.

## Current Smoke Path

1. `GET /healthz`
2. `GET /` at a 390 x 844 mobile viewport
3. `POST /app/start`
4. `POST /app/save-account`
5. `POST /app/approve`
6. `POST /app/reveal`
7. `POST /app/submit`
8. `POST /app/next`
9. `POST /app/logout`
10. JSON equivalent through `/accounts`, `/sources`, `/generate`, `/approve`,
   `/review/{review_unit_id}/reveal`, and `/review/{review_unit_id}/submit`

Submit payloads must include `idempotencyKey`.

## Local Browser Receipt

2026-06-06 local smoke: `MEMORY_ENGINE_API_STORE_DIR=.tmp/api-mobile-smoke
HOST=127.0.0.1 PORT=18081 cargo run -p memory-engine-api`, then Playwright
Chromium at `390 x 844` drove source-first home, generation, keep, reveal, and
submit with no horizontal overflow. The revealed answer
`CHARLIE ALFA TANGO` produced `Last result: Correct`.

2026-06-06 follow-up local smoke after the save/next UX slice: same command and
viewport drove source-first home, generation, account email save, keep, reveal,
submit using the revealed answer, and `Next review`. Every
checked page reported `scrollWidth == clientWidth == 390`.

2026-06-11 local hardening route proof: `cargo test -p memory-engine-api
-- --nocapture` covered `/app/account` rate limits by email and IP,
`/app/logout` CSRF enforcement, persisted browser-session revocation across a
file-backed router restart, and atomic file-store magic-link consumption. The
same suite continued to cover magic-link replay rejection, non-enumerating
login requests, allowlist enforcement, and Postgres browser-session resume.

2026-06-11 local binary smoke: with
`MEMORY_ENGINE_ENABLE_FILE_STORE=true MEMORY_ENGINE_API_STORE_DIR=.tmp/api-hardening-smoke MEMORY_ENGINE_AUTH_ALLOWED_EMAILS=owner@example.com MEMORY_ENGINE_AUTH_LINK_OUTBOX_PATH=.tmp/api-hardening-smoke/outbox.tsv HOST=127.0.0.1 PORT=18082 cargo run -p memory-engine-api`,
curl drove production-style magic-link login through the local outbox, verified
the link (`200`), posted `/app/logout` (`200` with `Max-Age=0`), retried
`/app/next` with the old cookie (`401`), then posted `/app/account` from one
client IP and saw attempts 1-5 return `200` and attempt 6 return `429`.

## Local Postgres Receipt

2026-06-06 local Postgres contract: with the existing
`sploot-test-postgres` container exposing `postgres://test:test@127.0.0.1:5432/sploot_test`,
this command passed:

```sh
MEMORY_ENGINE_POSTGRES_TEST_URL=postgres://test:test@127.0.0.1:5432/sploot_test cargo test -p memory-engine-persistence-postgres live_postgres_store_scopes_accounts_and_persists_idempotent_reviews -- --nocapture
```

The test creates an isolated schema, runs the adapter migration, writes account A
source documents, reference spans, generation runs, generated drafts, review
units, schedules, attempts, and applied-review receipts, then verifies duplicate
review idempotency, account A snapshot reconstruction, and account B isolation
before dropping the schema. It also runs `BetaStudySession` against an
account-scoped Postgres store for source intake, deterministic generation,
draft approval, reveal, graded submit, and persisted receipt reconstruction.

2026-06-06 local API/Postgres route contract: with the same
`sploot-test-postgres` container, this command passed:

```sh
MEMORY_ENGINE_POSTGRES_TEST_URL=postgres://test:test@127.0.0.1:5432/sploot_test cargo test -p memory-engine-api postgres_backend_routes_drive_source_to_review -- --nocapture
```

The test creates an isolated schema, starts the API with
`AccountRegistry::with_postgres_url`, drives JSON `/accounts`, `/sources`,
`/generate`, `/approve`, `/reveal`, and `/submit`, then recreates API state and
verifies source persistence after restart through the same Postgres schema.

## Fly Staging Receipt

2026-06-06 deployed `memory-engine-api` to Fly app
`https://memory-engine-api.fly.dev/` with Fly Managed Postgres cluster
`memory-engine-api-pg` (`nlkxjo56lnlry93v`) in `ord`. The app secret
`MEMORY_ENGINE_POSTGRES_URL` is attached to the app-specific database user
`memory-engine-api`; the initial default `fly-user` was downgraded to `reader`
after attach output exposed its connection URL in the local operator console.

Deploy command:

```sh
flyctl deploy -a memory-engine-api --remote-only
```

Deployed image:
`memory-engine-api:deployment-01KTF8VG6WT38CZDSKE9QKJN95`.

Fly Machines:

- `080395df316758` in `ord`
- `84e474a4266518` in `ord`

Health check:

```sh
curl -fsS https://memory-engine-api.fly.dev/healthz
```

returned:

```json
{"status":"ok","service":"memory-engine-api"}
```

JSON route smoke drove `POST /accounts`, `POST /accounts/{account_id}/sources`,
`POST /accounts/{account_id}/sources/{source_id}/generate`,
`POST /accounts/{account_id}/drafts/{draft_id}/approve`,
`POST /accounts/{account_id}/review/{review_unit_id}/reveal`,
`POST /accounts/{account_id}/review/{review_unit_id}/submit`, and
`GET /accounts/{account_id}/review/next`.

Receipt values:

```json
{
  "accountId": "acct_24d0e8dd75e01691",
  "sourceId": "src_333ca5cad6797ad8",
  "draftCount": 2,
  "reviewUnitId": "generated-quiz-src-333ca5cad6797ad8-1-nato-letter-a",
  "expectedAnswer": "ALFA",
  "verdict": "correct",
  "attemptCount": 1
}
```

The first deployed smoke exposed a production-only bug: with two Fly Machines,
account creation could land on one Machine and source creation on another,
where the in-memory API registry returned `Account not found`. The fix adds a
Postgres-backed API session table and validates `account_id + session_token`
from Postgres when a request lands on a fresh process. A follow-up critic pass
found the same durable-state issue for client review idempotency keys. The API
now passes the client idempotency key through the study/service write path and
short-circuits duplicate applied-review receipts from Postgres before grading.
The regression is covered by `postgres_backend_routes_drive_source_to_review`,
which creates a second `ApiState` before source creation and later resubmits
the same review idempotency key after API state recreation with the original
session token.

Restart/resume proof:

```sh
flyctl machine restart 84e474a4266518 -a memory-engine-api
flyctl machine restart 080395df316758 -a memory-engine-api
```

After both Machines restarted, listing sources with the original `account_id +
session_token` returned the persisted source:

```json
{
  "accountId": "acct_24d0e8dd75e01691",
  "sourceCount": 1,
  "resumedSourceId": "src_333ca5cad6797ad8",
  "resumedTitle": "NATO practice notes"
}
```

Mobile browser smoke used Chromium at `390 x 844` against the deployed URL and
drove source-first home, generation, account email save, keep, reveal, submit,
and final review state. Overflow checks reported
`scrollWidth == clientWidth == 390` on initial, generated, and final pages.

2026-06-06 follow-up deploy after the durable idempotency fix and
`min_machines_running = 1` config change used image
`memory-engine-api:deployment-01KTF8VG6WT38CZDSKE9QKJN95`. Deployed health
returned `{"status":"ok","service":"memory-engine-api"}`. The JSON route smoke
created `acct_ede61c543b71e396`, generated and approved
`generated-quiz-src-91fdb0ff98a73300-1-nato-letter-a`, submitted `ALFA`, then
recreated API state and submitted the same client idempotency key with the
original session token again. Both `attemptCount` and `duplicateAttemptCount`
were `1`, with
`duplicateLastOutcome: "correct"`.

## Remaining Production Gaps

- External auth is still a narrow app-owned session boundary, not a full
  passwordless/OAuth provider; the server-rendered no-JavaScript forms carry
  session tokens through hidden fields.
- Generation still uses deterministic source parsing; a provider-backed model
  adapter remains a future integration.
- The API opens a blocking Postgres client per operation and runs idempotent
  migrations lazily; add pooling/telemetry before higher-traffic use.
