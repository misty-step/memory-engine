# Learning Packet: Production Mobile Study App

## What Changed The Approach

The deployed Fly smoke mattered more than local confidence. Local API tests and
single-process route smoke passed, but production immediately exposed the
in-memory API session registry as invalid for two Machines. The fix was to move
only the API session validation needed for account routing into Postgres,
leaving learning semantics and persistence contracts in their existing
boundaries.

The closeout critic also caught that API idempotency was not the same as durable
service idempotency. The client idempotency key must reach the applied-review
receipt path; otherwise a process restart can bypass an API-local duplicate
cache.

## Codification Candidates

- Add an API QA lane that can intentionally recreate process state between
  account creation and source/review routes. This catches multi-Machine state
  assumptions before deploy.
- Keep duplicate-submit tests in the Postgres route contract whenever API
  idempotency behavior changes.
- Add a deployed smoke script under a repo-owned QA crate once this path repeats
  enough to justify automation. It should emit a redacted JSON receipt and
  avoid printing secrets.
- Consider a small Fly operations doc section for Managed Postgres user roles,
  because `flyctl mpg attach` can print a connection URL to the operator
  console.

## Backlog Candidates

- Integrate an external auth provider behind the existing API account/session
  boundary.
- Add a provider-backed generation adapter with model/version/cost/schema
  receipts while preserving deterministic generation for tests.
- Add Postgres pooling, structured request logs, and generation/review event
  telemetry.
- Add a first-class deployed QA command that exercises `/healthz`, account
  creation, source generation, approve, reveal, submit, next, restart/resume,
  and 390 x 844 overflow checks.

## Non-Actions

- Do not move auth, SQL, HTTP, provider clients, UI state, or deployment logic
  into `memory-engine-core`.
- Do not treat Fly volume JSON as production persistence for account-backed
  study state.
- Do not add native mobile scope until the mobile web flow has repeated
  dogfood evidence.
- Do not claim live AI generation quality until a provider adapter records
  model receipts and unsupported-claim checks.
