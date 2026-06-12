# Scry v1 API handoff

`memory-engine` owns memory science: source ingestion, draft generation,
review-unit approval, due queue selection, answer reveal, grading, scheduling,
and source archival. Scry owns the product experience: account UI, study
layout, navigation, copy, reminders, and client-side state.

External clients use only `/v1/...` JSON routes. Browser-only CSRF fields and
HTML forms are not part of this contract. Token clients authenticate with:

```text
Authorization: Bearer <sessionToken>
```

The session token comes from `POST /v1/accounts` or from a pre-provisioned
account session. The token is secret; receipts and demos must never print it.

## Contract Files

- `docs/api/openapi.v1.json` is the checked-in v1 contract.
- `GET /v1/openapi.json` serves the same contract from the API.
- `cargo test -p memory-engine-api v1_ -- --nocapture` proves the route table,
  OpenAPI methods, Bearer auth, and full machine-to-machine loop agree.
- `cargo test -p memory-engine-contract` starts the real API router on
  localhost, runs the external consumer runner over HTTP, fetches the served
  OpenAPI contract, completes the full loop, archives its source, and proves
  receipts redact credentials.
- `docs/qa/scry-v1-production-contract.md` records the production Fly contract
  receipt against a pre-provisioned account session.

## Consumer Demo

Create a disposable local or staging account where the email is allowlisted:

```sh
cargo run -p memory-engine-contract -- \
  --base-url http://127.0.0.1:18080 \
  --email scry-contract-local@example.com
```

Production account creation is allowlist-gated. The production demo must reuse
a pre-provisioned account without printing the session token:

```sh
MEMORY_ENGINE_ACCOUNT_ID=acct_... \
MEMORY_ENGINE_SESSION_TOKEN="$SESSION_TOKEN" \
cargo run -p memory-engine-contract -- \
  --base-url https://memory-engine-api.fly.dev
```

The runner creates a disposable source, generates drafts, approves the first
draft, selects the next review, reveals the answer, submits that answer,
archives the source, lists active sources, and emits a JSON receipt with the
source absent from the active list.

## Scry Integration Notes

Scry should treat `ReviewUnitId` as opaque. It should not infer concept,
phrasing, or scheduling meaning from IDs. The client can keep its own view
state, but the current due item, revealed answer, grade, attempt count,
post-answer feedback, item history, concept health rollup, and schedule result
come from the engine response.

After a submit, `current.feedback` carries human-language result text, the
expected answer, this item's attempt history (`lastSeen` plus
`lastSeenSummary`), and the matching concept rollup.
`conceptProgress` is the management-surface list of attempted concepts sorted
weakest first. It is derived from the existing attempt log and review-unit
concept metadata; clients should not build a separate analytics store for v1.

Use the OpenAPI file as Scry's source of truth for routes and schemas. If the
engine adds prompt enums, grader verdicts, or queue semantics, update the
contract, API tests, contract runner, and this handoff doc in the same change.
