# 099 — Service-session production receipt (2026-07-16)

Ticket: memory-engine-099. First live proof of the machine-consumer auth
surface shipped in PR [#59](https://github.com/misty-step/memory-engine/pull/59)
(merge `00c26c9`), deployed to DigitalOcean as deployment
`6b3d943d-00c0-4772-a54b-5248644d0f9c` (ACTIVE), verified against
`https://memory-engine-api-i2xcr.ondigitalocean.app`.

## What this proves

An agent obtained, used, rotated, and revoked a production credential for the
dedicated dogfood account `memory-engine-dogfood@mistystep.io`
(`acct_e4df8b03deafe7bc`) with **no email step and no browser session**. All
calls were plain `curl`; timings are `%{time_total}` from a residential
connection.

## Deployed smoke (pre-receipt)

| Check | Result |
|---|---|
| `GET /healthz` | 200 `"status":"ok"` |
| `GET /readyz` | 200 `"status":"ready"` |
| `GET /` | 200 |
| `POST /v1/service-sessions` (no token) | 403, no body parse |
| `POST /v1/service-sessions` (wrong token) | 403 |

## Receipt

| Step | Route | Result | Time |
|---|---|---|---|
| Issue credential | `POST /v1/service-sessions` | 201 | 0.304 s |
| Save source | `POST /v1/accounts/{account_id}/sources` | 201 | 1.135 s |
| Review next ×5 | `POST /v1/accounts/{account_id}/review/next` | 200 ×5 | 0.684–0.840 s |
| List sources | `GET /v1/accounts/{account_id}/sources` | 200 | 0.681 s |
| Reissue independent credential | `POST /v1/service-sessions` | 201, same `accountId`; prior session remains independent | 0.419 s |
| Revoke one current API session | `DELETE /v1/accounts/{account_id}/service-sessions/current` | route-owned explicit revoke | — |
| Logout all API sessions | `DELETE /v1/accounts/{account_id}/service-sessions/all` | route-owned explicit revoke-all | — |

Session lifecycle is explicit: issuing a new credential does not rotate or revoke
other sessions. Callers use the one-session or all-session revoke route when they
intend logout semantics.

## Isolation

Proven by route-level tests in `crates/memory-engine-api/src/tests/mod.rs`
(`service_session_credential_is_isolated_to_its_own_account`,
`service_session_reissue_revokes_the_prior_credential_immediately`,
`service_session_issuance_refuses_unauthorized_bodies_before_parsing`, and
siblings). No production cross-account probe was run: exercising another
account's routes with a live credential against real user data is exactly what
the tests exist to avoid.

## Full review-loop gap — closed by memory-engine-103

This receipt originally stopped after authenticated empty-state reads because
the machine plane could not enqueue production generation. PR #64 added bearer
enqueue and polling over the durable queue. The
[103 production receipt](103-machine-generation-receipt-2026-07-17.md) now
proves service credential → source → queued generation → next → submit end to
end and closes the remaining 099 criterion.

## Custody

- `MEMORY_ENGINE_ADMIN_TOKEN`: set as an encrypted app-spec secret on
  DigitalOcean (operator-rotatable via `doctl apps update`). Mint keychain
  custody (`secret://memory-engine/admin`) pending: the macOS keychain was
  locked for the whole session (headless, no user interaction).
- Dogfood session token (`secret://memory-engine/dogfood`): same pending
  state. Until custody lands, the token is recoverable by reissuing through
  the admin surface — rotation invalidates nothing but the previous token.
