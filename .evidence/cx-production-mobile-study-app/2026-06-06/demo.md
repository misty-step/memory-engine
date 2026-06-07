# Demo Proof: Production Mobile Study App

## Behavior Proved

A learner can use a phone-sized web app to submit source material, generate
cited study drafts, save or resume an account, keep generated material, reveal
an answer, submit a review, and continue to the next scheduled review. The same
account state survives Fly Machine restarts because production state is backed
by managed Postgres, not process memory or a local JSON file.

## Deployed Route Proof

The deployed Fly service passed:

```sh
curl -fsS https://memory-engine-api.fly.dev/healthz
```

with:

```json
{"status":"ok","service":"memory-engine-api"}
```

The JSON smoke drove these routes against `https://memory-engine-api.fly.dev`:

- `POST /accounts`
- `POST /accounts/{account_id}/sources`
- `POST /accounts/{account_id}/sources/{source_id}/generate`
- `POST /accounts/{account_id}/drafts/{draft_id}/approve`
- `POST /accounts/{account_id}/review/{review_unit_id}/reveal`
- `POST /accounts/{account_id}/review/{review_unit_id}/submit`
- `GET /accounts/{account_id}/review/next`

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

## Restart/Resume Proof

Commands exercised:

```sh
flyctl machine restart 84e474a4266518 -a memory-engine-api
flyctl machine restart 080395df316758 -a memory-engine-api
```

After restart, listing sources with the original `account_id + session_token`
returned:

```json
{
  "accountId": "acct_24d0e8dd75e01691",
  "sourceCount": 1,
  "resumedSourceId": "src_333ca5cad6797ad8",
  "resumedTitle": "NATO practice notes"
}
```

## Mobile Render Proof

Chromium at `390 x 844` against the deployed URL drove source-first home,
generation, account email save, keep, reveal, submit, and final review state.

Overflow checks reported:

```json
{
  "initialOverflow": {"scrollWidth": 390, "clientWidth": 390, "title": "Memory Engine Study"},
  "generatedOverflow": {"scrollWidth": 390, "clientWidth": 390, "hasGenerated": true, "hasKeep": true},
  "finalOverflow": {"scrollWidth": 390, "clientWidth": 390, "hasCorrect": true, "hasNext": true}
}
```

## Production Bug Found And Fixed

The first deployed smoke exposed a production-only bug: two Fly Machines meant
account creation could land on one Machine and source creation on another,
where the in-memory API registry returned `Account not found`.

The fix persists API sessions in Postgres and validates `account_id +
session_token` from Postgres when a request lands on a fresh process. The
regression test `postgres_backend_routes_drive_source_to_review` creates a
second `ApiState` before source creation, simulating a different Machine.

A follow-up critic pass found the same production-state problem for review
idempotency. The API now passes the client idempotency key into the service
attempt and checks Postgres for an already-applied review receipt before grading
again. The deployed follow-up smoke used account `acct_ede61c543b71e396` and
review unit `generated-quiz-src-91fdb0ff98a73300-1-nato-letter-a`; resubmitting
the same client key after API state recreation with the original session token
returned `duplicateAttemptCount: 1` and `duplicateLastOutcome: "correct"`.

## Residual Risk

- External auth remains a narrow app-owned account/session boundary.
- Generation remains deterministic source parsing with a provider seam; a live
  model adapter is still future work.
- The API uses a blocking Postgres client per operation and lazy idempotent
  migrations; pooling and telemetry should precede higher-traffic use.
- Fly Managed Postgres is a live paid resource.
