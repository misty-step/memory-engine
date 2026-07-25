# MCP Review Loop Dogfood

Refs-Powder: memory-engine-071, memory-engine-mcp-production-parity

## Purpose

`crates/memory-engine-mcp` is Scry's MCP face: a stdio
JSON-RPC server wrapping the deployed `memory-engine-api` v1 contract, so any
MCP-speaking agent (Claude Code, Codex, etc.) can run a study/review loop and
manage project decks without shelling out to `memory-engine-review` or
speaking raw HTTP. It closes a cell in the fleet-wide "five faces" coverage
matrix (skill / CLI / API / MCP / UI) alongside `memory-engine-review` (CLI,
070) and `memory-engine-api` (API).

It adds **no new server surface**. Every tool composes one or more existing
v1 routes; `crates/memory-engine-mcp/src/client.rs` is the only place that
speaks HTTP. Generation is queue-based end to end: `create_deck` enqueues a
durable generation job (`POST .../generation-jobs`) and polls it
(`GET .../generation-jobs/{id}`) to a bounded terminal state, never the
legacy synchronous `POST .../generate` route — that route is refused
outright with HTTP 409 in every production deployment once
`MEMORY_ENGINE_POSTGRES_URL` is set (`registry.rs::generate_source`). A
`succeeded` job's accepted drafts remain pending until an explicit
`keep_draft`, `edit_draft`, or `reject_draft` decision — generation never
schedules a card by itself, a policy shared with every other caller
(including the browser UI) that this MCP face leaves unchanged.

## Tool contract

Tools are agent-intent-shaped, not 1:1 REST wrappers — the discipline
`powder-mcp` established for this fleet (`../../Development/powder/crates/powder-mcp`
was the reference exemplar for the stdio transport, tool registration, and
error-handling shape).

| Tool | Composes | Intent |
|---|---|---|
| `create_deck` | `POST project-decks` -> `POST sources/{id}/generation-jobs` -> bounded poll `GET generation-jobs/{id}` | Capture material as a study deck and drive it through the durable production generation queue to a bounded terminal state; accepted drafts remain pending for an explicit decision. |
| `keep_draft` / `edit_draft` / `reject_draft` | `POST drafts/{id}/keep`, `/edit`, or `/reject` | Make an explicit learner decision; only keep or edit-and-keep creates a due card, while reject remains terminal and exportable. |
| `list_decks` | `GET sources`, filtered to `projectKey.is_some()` | Check what decks exist, or find a `deck_id` to invalidate. |
| `invalidate_deck` | `POST project-decks/{id}/invalidate` | Retire every card from one deck after an external event (029's project-deck lifecycle). |
| `list_drafts` | `POST review/next`, filtered to accepted-and-undecided | Inspect every pending draft across the account, independent of which `create_deck` call produced it — the normal state right after generation, before a keep/edit/reject decision. |
| `list_due` | `POST review/next` | Lightweight status check: how many are due, one-line teaser of the next prompt. |
| `review_next` | `POST review/next` | Full detail (prompt, choices, `review_unit_id`) to actually answer. |
| `submit_answer` | `POST review/{id}/submit` | Grade an answer and advance the schedule. |
| `reveal_answer` | `POST review/{id}/reveal` | Declared remediation: show the expected answer without grading. |
| `learn_more` | `POST review/{id}/reference` | Declared remediation: request extra reference material instead of grading now. |
| `skip_review` | `POST review/{id}/skip` | Declared remediation: skip this card for this pass, schedule untouched. |
| `snooze_review` | `POST review/{id}/snooze` | Declared remediation: push just this card later in the due queue. |
| `snooze_concept` | `POST review/{id}/snooze-concept` | Declared remediation: push every card for this card's concept later in the due queue. |
| `bridge_review` | `POST review/{id}/bridge` | Declared remediation: request bridge (scaffold) material for a card the learner keeps missing. |
| `record_content_feedback` | `POST review/{id}/content-feedback` | Record a kept/dropped verdict on the generated content itself, distinct from grading an answer. |

`list_due` and `review_next` both call the same v1 route (`review/next` is
the only v1 route that returns due state — there is no separate read-only
"study view" route) but shape the response differently: `list_due` is a
peek, `review_next` is the full payload needed to work the card. Two
distinct agent intents over one endpoint, same discipline as
`next_app_review`/`app_study_view` in `memory-engine-api-state` calling
different backend methods for different UI intents.

## Credential model

`memory-engine-review` and `memory-engine-mcp` resolve credentials through
the shared `memory-engine-credentials` crate, so `memory-engine-review
login` and a freshly started `memory-engine-mcp` agree on the same account
without an operator manually copying a credentials file between the two
clients' state directories (each previously wrote its own subdirectory, so
logging in with one did not make the other work). Resolution order: env vars
(`MEMORY_ENGINE_ACCOUNT_ID` / `MEMORY_ENGINE_SESSION_TOKEN`) -> the one
shared credentials file (`$MEMORY_ENGINE_HOME/credentials.json`,
`~/.memory-engine/credentials.json` when unset, migrating a legacy
per-client file into it first) -> fail loudly. Credentials must be
provisioned through the invite magic-link or operator service-session flow
before this stdio server starts; anonymous account creation is disabled and
there is no email-bootstrap fallback. There is no interactive `login`
subcommand for the MCP server either — stdin is the JSON-RPC channel, not a
terminal.

There is no in-memory fallback
(`crates/memory-engine-mcp/tests/no_credentials_fallback.rs` proves the
server exits non-zero with no stdout before reading any stdin when no
credential path resolves) — the same lesson `powder-mcp` encoded after an
ephemeral in-memory mode silently evaporated claims on process exit.

`MEMORY_ENGINE_MCP_BASE_URL` overrides the base URL. The default is the
branded production origin `https://scry.study`. The DigitalOcean App
Platform origin `https://memory-engine-api-i2xcr.ondigitalocean.app` that
`scry.study` fronts is still live and still the identity `docs/runbook.md`'s
deploy smoke checks against, but it is only an operator-origin fallback now
— pass it explicitly via `--base-url` / `MEMORY_ENGINE_MCP_BASE_URL` while
DNS for the branded domain is degraded, never as a silent default.

## Commands

```sh
cargo build -p memory-engine-mcp
cargo test -p memory-engine-mcp
MEMORY_ENGINE_ACCOUNT_ID=... MEMORY_ENGINE_SESSION_TOKEN=... \
  MEMORY_ENGINE_MCP_BASE_URL=https://scry.study \
  ./target/debug/memory-engine-mcp
```

## Falsifier

**Claim:** a cold agent (no prior context, no memory-engine-specific code)
can drive a full deck-create -> inspect -> review -> remediation -> feedback
-> invalidate loop purely over MCP stdio JSON-RPC, using only the tool
descriptions returned by `tools/list`, and generation always reaches the
production job queue rather than the retired synchronous route.

**Would falsify it:** the loop cannot complete without reaching into
`memory-engine-api` internals, a tool silently drops server state between
calls, a tool is a bare REST-route echo instead of an agent intent, or
`create_deck` ever calls the legacy synchronous `/generate` route.

## Automated coverage

- `crates/memory-engine-mcp/src/lib.rs` unit tests: tool contract shape
  (seventeen agent-intent tools, no REST-route names), schema validity, and
  argument validation.
- `crates/memory-engine-mcp/src/session.rs` unit tests: credential
  resolution precedence (shared with `memory-engine-review`, including a
  one-time legacy-file migration to the shared default) and the
  no-fallback failure path.
- `crates/memory-engine-mcp/src/client.rs` unit tests: the queued
  generation-jobs composition, a behavioral HTTP-request-capture proof
  (`create_deck_enqueues_and_polls_without_ever_requesting_generate`) that
  `create_deck` never requests the legacy `/generate` route and does
  enqueue+poll (leaving exactly one draft pending, never auto-scheduled),
  and a draft generated through the local (non-production) synchronous
  route can still be inspected (`list_drafts`) and kept
  (`keep_draft`) explicitly.
- `crates/memory-engine-mcp/tests/no_credentials_fallback.rs`: spawns the
  real compiled binary with no credential path configured; asserts non-zero
  exit and no stdout.
- `crates/memory-engine-mcp/tests/stdio_review_loop.rs`: spawns the real
  compiled binary against a real local `memory-engine-api` (in-process axum
  server with its background generation worker started, not a mock) and
  drives a twelve-call sequence over its actual stdin/stdout pipes,
  asserting the queued job reaches `succeeded` with a pending draft,
  `keep_draft` schedules it, `dueCount` transitions 1 -> 0, and the deck
  stays retired after invalidation.
- `crates/memory-engine-mcp/tests/postgres_production_parity.rs`: the same
  queued composition against a Postgres-backed `ApiState` (the same backend
  selection every real production deployment makes), reproducing the
  reported HTTP 409 on the legacy route and proving the queued fix reaches
  `succeeded` on Postgres. Skipped unless `MEMORY_ENGINE_POSTGRES_TEST_URL`
  points at a scratch database.

## Self-run transcript

A local `memory-engine-api` was started exactly per `docs/runbook.md`'s
local file-store pattern (the same Rust binary that runs in production,
pointed at a scratch store dir; the binary always starts its background
generation worker before serving). `MEMORY_ENGINE_ENVIRONMENT=development`
marks the run as local so the production `MEMORY_ENGINE_ADMIN_TOKEN`
requirement does not apply:

```sh
$ MEMORY_ENGINE_ENVIRONMENT=development \
  MEMORY_ENGINE_ENABLE_FILE_STORE=true \
  MEMORY_ENGINE_API_STORE_DIR=/tmp/me-mcp-dogfood/store \
  MEMORY_ENGINE_AUTH_ALLOWED_EMAILS=dogfood-mcp@example.com \
  MEMORY_ENGINE_AUTH_LINK_OUTBOX_PATH=/tmp/me-mcp-dogfood/outbox.tsv \
  MEMORY_ENGINE_RETURN_UNSUBSCRIBE_SECRET=local-dogfood-secret \
  HOST=127.0.0.1 PORT=18199 ./target/debug/memory-engine-api &
Memory Engine API listening on http://127.0.0.1:18199

$ curl -sS -X POST http://127.0.0.1:18199/v1/accounts \
    -H 'content-type: application/json' -d '{"email":"dogfood-mcp@example.com"}'
{"accountId":"acct_55c1c8328e564d09","sessionToken":"sess_***redacted***"}
```

Then the actual `memory-engine-mcp` binary — not `cargo test`, a real
process fed real JSON-RPC lines on stdin, one line of output per line of
input:

```sh
$ MEMORY_ENGINE_MCP_BASE_URL=http://127.0.0.1:18199 \
  MEMORY_ENGINE_ACCOUNT_ID=acct_55c1c8328e564d09 \
  MEMORY_ENGINE_SESSION_TOKEN=sess_*** \
  ./target/debug/memory-engine-mcp
memory-engine-mcp: ready (account acct_55c1c8328e564d09, base url http://127.0.0.1:18199)
```

**1. `initialize`**

```json
>>> {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}
<<< {"id":1,"jsonrpc":"2.0","result":{"capabilities":{"tools":{"listChanged":false}},"protocolVersion":"2024-11-05","serverInfo":{"name":"memory-engine","version":"0.0.0"}}}
```

**2. `tools/list`** — seventeen agent-intent tools (truncated here; full
output is identical to the table above rendered as JSON Schema):

```json
>>> {"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}
<<< {"id":2,"jsonrpc":"2.0","result":{"tools":[{"name":"create_deck",...},{"name":"keep_draft",...},{"name":"edit_draft",...},{"name":"reject_draft",...},{"name":"list_decks",...},{"name":"invalidate_deck",...},{"name":"list_drafts",...},{"name":"list_due",...},{"name":"review_next",...},{"name":"submit_answer",...},{"name":"reveal_answer",...},{"name":"learn_more",...},{"name":"skip_review",...},{"name":"snooze_review",...},{"name":"snooze_concept",...},{"name":"bridge_review",...},{"name":"record_content_feedback",...}]}}
```

**3. `create_deck`** — saves the source and drives it through the durable
generation-jobs queue (enqueue + bounded poll), never the legacy synchronous
`/generate` route. `generation.job.cardCount: 0` proves generation never
auto-schedules; the one accepted draft comes back in `generation.pendingDrafts`
for an explicit decision:

```json
>>> {"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"create_deck","arguments":{"project_key":"nato-onboarding","title":"NATO letter A fixture","body":"Concept: NATO letter A\nActivity: quiz\nStage: recognition-3\nQuestion: What is the NATO phonetic alphabet word for A?\nAnswer: ALFA\nDistractors: BRAVO, CHARLIE\nReference: The NATO phonetic alphabet word for A is ALFA."}}}
<<< {"id":3,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"{\n  \"deck\": {\"deckId\": \"deck_80cb605a305a472e\", \"projectKey\": \"nato-onboarding\", \"source\": {\"sourceId\": \"deck_80cb605a305a472e\", \"title\": \"NATO letter A fixture\", ...}},\n  \"generation\": {\"coalesced\": false, \"job\": {\"id\": \"job-0bf061821d32534891018729e22c710f\", \"status\": \"succeeded\", \"cardCount\": 0, \"error\": null, ...}, \"pendingDrafts\": [{\"id\": \"file-job-job-0bf061821d32534891018729e22c710f-draft-deck-80cb605a305a472e-1-nato-letter-a\", \"reviewUnitId\": \"generated-quiz-deck-80cb605a305a472e-1-nato-letter-a\", \"validationStatus\": \"accepted\", \"approved\": false, \"learnerDecision\": null, \"sourceSpans\": [...], \"provenance\": {...}, ...}], \"status\": \"succeeded\"}\n}"}]}}
```

**4. `list_drafts`** — the same pending draft is visible account-wide,
independent of the `create_deck` response above:

```json
>>> {"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"list_drafts","arguments":{}}}
<<< {"id":4,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"[{\"id\": \"file-job-job-0bf061821d32534891018729e22c710f-draft-deck-80cb605a305a472e-1-nato-letter-a\", \"validationStatus\": \"accepted\", \"approved\": false, \"learnerDecision\": null, ...}]"}]}}
```

**5. `keep_draft`** — the explicit learner decision; only this schedules a
review card, and `dueCount` becomes 1:

```json
>>> {"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"keep_draft","arguments":{"draft_id":"file-job-job-0bf061821d32534891018729e22c710f-draft-deck-80cb605a305a472e-1-nato-letter-a"}}}
<<< {"id":5,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"{\n  \"current\": {\"reviewUnitId\": \"generated-quiz-deck-80cb605a305a472e-1-nato-letter-a\", \"prompt\": \"What is the NATO phonetic alphabet word for A?\", \"choices\": [\"BRAVO\", \"CHARLIE\", \"ALFA\"], ...},\n  \"drafts\": [{\"approved\": true, \"learnerDecision\": {\"kind\": \"kept\", \"edited\": false, ...}, ...}],\n  \"dueCount\": 1, ...\n}"}]}}
```

**6. `list_decks`** — the new deck is visible, scoped to its `project_key`:

```json
>>> {"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"list_decks","arguments":{"project_key":"nato-onboarding"}}}
<<< {"id":6,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"[{\"sourceId\": \"deck_80cb605a305a472e\", \"projectKey\": \"nato-onboarding\", \"title\": \"NATO letter A fixture\", ...}]"}]}}
```

**7. `list_due`** — a lightweight peek, one due card:

```json
>>> {"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"list_due","arguments":{}}}
<<< {"id":7,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"{\n  \"dueCount\": 1,\n  \"nextPrompt\": \"What is the NATO phonetic alphabet word for A?\"\n}"}]}}
```

**8. `review_next`** — full detail, including `reviewUnitId` needed to
answer:

```json
>>> {"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"review_next","arguments":{}}}
<<< {"id":8,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"{\n  \"current\": {\n    \"choices\": [\"BRAVO\", \"CHARLIE\", \"ALFA\"],\n    \"prompt\": \"What is the NATO phonetic alphabet word for A?\",\n    \"reviewUnitId\": \"generated-quiz-deck-80cb605a305a472e-1-nato-letter-a\", ...\n  },\n  \"dueCount\": 1, ...\n}"}]}}
```

**9. `reveal_answer`** — declared remediation: show the expected answer
without grading, and the queue stays on the same card:

```json
>>> {"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"reveal_answer","arguments":{"review_unit_id":"generated-quiz-deck-80cb605a305a472e-1-nato-letter-a"}}}
<<< {"id":9,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"{\n  \"current\": {\n    \"expectedAnswer\": \"ALFA\", \"reviewUnitId\": \"generated-quiz-deck-80cb605a305a472e-1-nato-letter-a\", ...\n  },\n  \"dueCount\": 1, ...\n}"}]}}
```

**10. `submit_answer`** — correct answer, grades the card, advances the
schedule, `dueCount` drops to 0:

```json
>>> {"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"submit_answer","arguments":{"review_unit_id":"generated-quiz-deck-80cb605a305a472e-1-nato-letter-a","answer":"ALFA"}}}
<<< {"id":10,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"{\n  \"current\": {\n    \"grade\": {\"isCorrect\": true, \"rating\": 3, \"verdict\": \"correct\"},\n    \"scheduleChange\": {\"before\": null, \"after\": {\"reps\": 1, \"lapses\": 0, ...}}, ...\n  },\n  \"dueCount\": 0, ...\n}"}]}}
```

**11. `record_content_feedback`** — a kept/dropped verdict on the content
itself, distinct from grading the answer:

```json
>>> {"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"record_content_feedback","arguments":{"review_unit_id":"generated-quiz-deck-80cb605a305a472e-1-nato-letter-a","verdict":"kept","rationale":"clear and correct"}}}
<<< {"id":11,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"{\n  \"verdict\": \"kept\",\n  \"reviewUnitId\": \"generated-quiz-deck-80cb605a305a472e-1-nato-letter-a\", ...\n}"}]}}
```

**12. `invalidate_deck`** — retires the deck; due count stays at 0:

```json
>>> {"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"invalidate_deck","arguments":{"deck_id":"deck_80cb605a305a472e","event":"onboarding project closed"}}}
<<< {"id":12,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"{\n  \"current\": null,\n  \"drafts\": [],\n  \"dueCount\": 0, ...\n}"}]}}
```

This is the same twelve-call sequence `stdio_review_loop.rs` asserts
automatically; the run above is a hand-driven capture from a real terminal
against the queued generation-jobs and explicit-keep-decision path, not test
output.

## Factory MCP registry entry

The canonical registry for bootstrapped-agent MCP visibility is
`~/.harness-kit/factory-mcps.yaml` (Harness Kit, `crates/harness-kit-checks/src/mcp_registry.rs`).
That file is out of scope for this ticket (a different repo, its own review
process), but the entry this server would need — matching the shape
`powder`'s entry already uses — is:

```yaml
- id: memory-engine
  app: Scry
  source_repo: misty-step/scry
  product_skill: memory-engine   # or whatever repo-local skill wraps this product
  status: available
  capabilities:
    - spaced-repetition review loops
    - project-deck capture and invalidation
    - study due-queue status
  scope:
    default_profiles: []   # opt-in per project, not global like canary
    include_repo_globs: ["/Users/phaedrus/Development/memory-engine/**"]
    exclude_repo_globs: []
  required_env_any:
    - [MEMORY_ENGINE_ACCOUNT_ID, MEMORY_ENGINE_SESSION_TOKEN]
  env_sources:
    - name: MEMORY_ENGINE_ACCOUNT_ID
      op_ref: op://Agents/MEMORY_ENGINE_ACCOUNT_ID/credential
    - name: MEMORY_ENGINE_SESSION_TOKEN
      op_ref: op://Agents/MEMORY_ENGINE_SESSION_TOKEN/credential
  codex:
    server_name: memory-engine
    command: bash
    args:
      - -lc
      - test -n "${MEMORY_ENGINE_ACCOUNT_ID:-}" || MEMORY_ENGINE_ACCOUNT_ID="$(op read "op://Agents/MEMORY_ENGINE_ACCOUNT_ID/credential")"; test -n "${MEMORY_ENGINE_SESSION_TOKEN:-}" || MEMORY_ENGINE_SESSION_TOKEN="$(op read "op://Agents/MEMORY_ENGINE_SESSION_TOKEN/credential")"; export MEMORY_ENGINE_ACCOUNT_ID MEMORY_ENGINE_SESSION_TOKEN; cd /Users/phaedrus/Development/memory-engine && exec cargo run --locked -q -p memory-engine-mcp
    env_policy: inherited_or_op_agents_vault
```

Registering it for real is a fast follow-up in the `harness-kit` repo (or
`~/.harness-kit` directly), not this ticket — it is a cross-repo, live
agent-config change with its own review surface.
