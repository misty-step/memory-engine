# 103 — Machine queued-generation production receipt (2026-07-17)

Ticket: memory-engine-103. Production proof of the bearer-authenticated queued
generation surface shipped in PR [#64](https://github.com/misty-step/memory-engine/pull/64)
(merge `f5a831d`), deployed to DigitalOcean as deployment
`ba8e87a5-31ee-475b-b019-27f97c20c76e` (ACTIVE). The credential-rotation app
update completed as deployment `8295334e-9518-43be-9f35-3813ae2c39f8` (ACTIVE).
All requests targeted `https://memory-engine-api-i2xcr.ondigitalocean.app`.

## What this proves

A machine consumer used a service-session bearer credential to save source
material, enqueue the same durable generation job used by the browser, poll it
to completion, receive a scheduled review unit, and submit a correct answer.
No browser session, cookie, CSRF token, or synchronous generation route was
used. This closes the open full-loop criterion from the
[099 service-session receipt](099-service-session-receipt-2026-07-16.md).

## Production receipt

| Step | Route | Result | Time |
|---|---|---|---|
| Liveness | `GET /healthz` | 200 | 0.173 s |
| Readiness | `GET /readyz` | 200 | 0.294 s |
| Issue service credential | `POST /v1/service-sessions` | 201; account `acct_e4df8b03deafe7bc` | 0.392 s |
| Save source | `POST /v1/accounts/{account_id}/sources` | 201; source `src_1a8cd4c00482e36f` | 1.260 s |
| Enqueue generation | `POST /v1/accounts/{account_id}/sources/{source_id}/generation-jobs` | 202; job `job-443d77e8aaf0b99ff4523286c91a0d6a` | 1.019 s |
| Poll completion | `GET /v1/accounts/{account_id}/generation-jobs/{job_id}` | 200 ×6; final `succeeded`, `cardCount: 1` | six polls |
| Select next review | `POST /v1/accounts/{account_id}/review/next` | 200; `generated-quiz-src-1a8cd4c00482e36f-1-nato-phonetic-alphabet` | 0.854 s |
| Submit `ALFA` | `POST /v1/accounts/{account_id}/review/{review_unit_id}/submit` | 200; verdict `correct`, attempt count 1 | 1.640 s |
| Archive proof source | `DELETE /v1/accounts/{account_id}/sources/{source_id}` | 204 | 1.942 s |
| Reissue and discard credential | `POST /v1/service-sessions` | 201 | 0.304 s |
| Probe revoked credential | `POST /v1/accounts/{account_id}/review/next` | 403 | 0.291 s |

The source used the deterministic structured-block contract:

```text
Concept: NATO phonetic alphabet
Question: What is the NATO code word for A?
Answer: ALFA
Reference: ALFA is the NATO code word for A.
```

## Security and cleanup

- The dogfood session credential was revoked by reissuing and discarding its
  replacement; the prior token returned 403 immediately.
- The proof source and its generated review material were archived after the
  successful submission.
- `MEMORY_ENGINE_ADMIN_TOKEN` was rotated as an encrypted DigitalOcean app-spec
  secret before issuance. The temporary local app spec was mode `0600`, then
  deleted; the plaintext token was cleared from the execution process.
- No credential or secret value is present in this receipt.
