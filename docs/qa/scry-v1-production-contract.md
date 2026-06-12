# QA receipt - Scry v1 production contract

Date: 2026-06-12. Ticket: backlog.d/_done/049-scry-consumable-api.md.
Production: `https://memory-engine-api.fly.dev`.

## Live Verification

The external contract client ran against the deployed Fly API using the
pre-provisioned production account `acct_48e443e2719d6f90`. The session token
came from Neon inside the shell process and was not printed.

Command shape:

```sh
MEMORY_ENGINE_ACCOUNT_ID=acct_48e443e2719d6f90 \
MEMORY_ENGINE_SESSION_TOKEN="$SESSION_TOKEN" \
cargo run -p memory-engine-contract -- \
  --base-url https://memory-engine-api.fly.dev
```

Sanitized receipt:

```json
{
  "baseUrl": "https://memory-engine-api.fly.dev",
  "openapiVersion": "3.1.0",
  "contractPathCount": 9,
  "accountId": "acct_48e443e2719d6f90",
  "createdAccount": false,
  "sourceId": "src_ee706c82993d0700",
  "draftId": "study-run-2-draft-src-ee706c82993d0700-1-nato-letter-a",
  "reviewUnitId": "generated-quiz-src-ee706c82993d0700-1-nato-letter-a",
  "verdict": "correct",
  "attemptCount": 1,
  "archivedSource": true,
  "activeSourcePresentAfterArchive": false
}
```

## Acceptance

- `GET /v1/openapi.json` served OpenAPI `3.1.0`.
- The contract exposed all Scry-required paths.
- The runner created a disposable source, generated study drafts, approved the
  first draft, selected the next due review, revealed the answer, submitted the
  revealed answer, and received verdict `correct`.
- The runner archived the source and proved it was absent from active sources.

## Residual Risk

This verifies the API contract with an existing production session. It does not
verify production self-service account creation, which remains allowlist-gated
and belongs to the browser magic-link flow.
