# MCP Review Loop Dogfood

Refs-Powder: memory-engine-071, memory-engine-mcp-production-parity

## Purpose

`crates/memory-engine-mcp` is the MCP face for `memory-engine`: a stdio
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
`succeeded` job's accepted cards arrive already scheduled: the production job
runner optimistically approves every accepted draft as part of the job
itself, a policy shared with every other caller (including the browser UI)
that this MCP face leaves unchanged.

## Tool contract

Tools are agent-intent-shaped, not 1:1 REST wrappers — the discipline
`powder-mcp` established for this fleet (`../../Development/powder/crates/powder-mcp`
was the reference exemplar for the stdio transport, tool registration, and
error-handling shape).

| Tool | Composes | Intent |
|---|---|---|
| `create_deck` | `POST project-decks` -> `POST sources/{id}/generation-jobs` -> bounded poll `GET generation-jobs/{id}` | Capture material as a study deck and drive it through the durable production generation queue to a bounded terminal state. |
| `list_decks` | `GET sources`, filtered to `projectKey.is_some()` | Check what decks exist, or find a `deck_id` to invalidate. |
| `invalidate_deck` | `POST project-decks/{id}/invalidate` | Retire every card from one deck after an external event (029's project-deck lifecycle). |
| `list_drafts` | `POST review/next`, filtered to accepted-and-unapproved | Inspect drafts stuck without a decision (a partially-committed job, or a legacy-origin draft) — usually empty, since a normal job already schedules its own accepted cards. |
| `approve_draft` | `POST drafts/{id}/approve` | Explicitly keep one pending draft, scheduling it as a live review card — a recovery/completion action, not a required follow-up to every generation. |
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
`~/.memory-engine/credentials.json` when unset) -> `MEMORY_ENGINE_MCP_EMAIL`
bootstrap -> fail loudly. There is no interactive `login` subcommand for the
MCP server — stdin is the JSON-RPC channel, not a terminal — so a brand-new
local server instead bootstraps its own account non-interactively from
`MEMORY_ENGINE_MCP_EMAIL`, persisting the result to the shared file (mode
`0600`) for reuse across restarts, including by `memory-engine-review`.

There is no in-memory fallback
(`crates/memory-engine-mcp/tests/no_credentials_fallback.rs` proves the
server exits non-zero with no stdout before reading any stdin when none of
the three resolve) — the same lesson `powder-mcp` encoded after an
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
  (fifteen agent-intent tools, no REST-route names), schema validity,
  argument validation, and a structural guard
  (`create_deck_never_calls_the_legacy_synchronous_generate_route`) that
  `create_deck` never touches `/generate`.
- `crates/memory-engine-mcp/src/session.rs` unit tests: credential
  resolution precedence (shared with `memory-engine-review`) and the
  no-fallback failure path.
- `crates/memory-engine-mcp/src/client.rs` unit tests: the queued
  generation-jobs composition, and a legacy-origin unapproved draft can
  still be inspected (`list_drafts`) and approved (`approve_draft`).
- `crates/memory-engine-mcp/tests/no_credentials_fallback.rs`: spawns the
  real compiled binary with no credential path configured; asserts non-zero
  exit and no stdout.
- `crates/memory-engine-mcp/tests/stdio_review_loop.rs`: spawns the real
  compiled binary against a real local `memory-engine-api` (in-process axum
  server with its background generation worker started, not a mock) and
  drives an eleven-call sequence over its actual stdin/stdout pipes,
  asserting the queued job reaches `succeeded`, `dueCount` transitions
  1 -> 0, and the deck stays retired after invalidation.
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
generation worker before serving):

```sh
$ MEMORY_ENGINE_ENABLE_FILE_STORE=true \
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

**2. `tools/list`** — fifteen agent-intent tools (truncated here; full
output is identical to the table above rendered as JSON Schema):

```json
>>> {"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}
<<< {"id":2,"jsonrpc":"2.0","result":{"tools":[{"name":"create_deck",...},{"name":"list_decks",...},{"name":"invalidate_deck",...},{"name":"list_drafts",...},{"name":"approve_draft",...},{"name":"list_due",...},{"name":"review_next",...},{"name":"submit_answer",...},{"name":"reveal_answer",...},{"name":"learn_more",...},{"name":"skip_review",...},{"name":"snooze_review",...},{"name":"snooze_concept",...},{"name":"bridge_review",...},{"name":"record_content_feedback",...}]}}
```

**3. `create_deck`** — saves the source and drives it through the durable
generation-jobs queue (enqueue + bounded poll), never the legacy synchronous
`/generate` route. `generation.status: "succeeded"` and `pendingDrafts: []`
prove the queue's optimistic approval already scheduled the card:

```json
>>> {"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"create_deck","arguments":{"project_key":"nato-onboarding","title":"NATO letter A fixture","body":"Concept: NATO letter A\nActivity: quiz\nStage: recognition-3\nQuestion: What is the NATO phonetic alphabet word for A?\nAnswer: ALFA\nDistractors: BRAVO, CHARLIE\nReference: The NATO phonetic alphabet word for A is ALFA."}}}
<<< {"id":3,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"{\n  \"deck\": {\n    \"deckId\": \"deck_80cb605a305a472e\",\n    \"projectKey\": \"nato-onboarding\",\n    \"source\": {\"sourceId\": \"deck_80cb605a305a472e\", \"title\": \"NATO letter A fixture\", ...}\n  },\n  \"generation\": {\n    \"coalesced\": false,\n    \"job\": {\"id\": \"job-8fdbfe53281539800c291d3444c95e34\", \"status\": \"succeeded\", \"cardCount\": 1, \"error\": null, ...},\n    \"pendingDrafts\": [],\n    \"status\": \"succeeded\"\n  }\n}"}]}}
```

**4. `list_drafts`** — inspect: nothing pending, because the job above
already committed its own accepted draft:

```json
>>> {"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"list_drafts","arguments":{}}}
<<< {"id":4,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"[]"}]}}
```

**5. `list_due`** — a lightweight peek, one due card:

```json
>>> {"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"list_due","arguments":{}}}
<<< {"id":5,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"{\n  \"dueCount\": 1,\n  \"nextPrompt\": \"What is the NATO phonetic alphabet word for A?\"\n}"}]}}
```

**6. `review_next`** — full detail, including `reviewUnitId` needed to
answer:

```json
>>> {"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"review_next","arguments":{}}}
<<< {"id":6,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"{\n  \"current\": {\n    \"choices\": [\"BRAVO\", \"CHARLIE\", \"ALFA\"],\n    \"prompt\": \"What is the NATO phonetic alphabet word for A?\",\n    \"reviewUnitId\": \"generated-quiz-deck-80cb605a305a472e-1-nato-letter-a\", ...\n  },\n  \"dueCount\": 1, ...\n}"}]}}
```

**7. `reveal_answer`** — declared remediation: show the expected answer
without grading, and the queue stays on the same card:

```json
>>> {"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"reveal_answer","arguments":{"review_unit_id":"generated-quiz-deck-80cb605a305a472e-1-nato-letter-a"}}}
<<< {"id":7,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"{\n  \"current\": {\n    \"expectedAnswer\": \"ALFA\", \"reviewUnitId\": \"generated-quiz-deck-80cb605a305a472e-1-nato-letter-a\", ...\n  },\n  \"dueCount\": 1, ...\n}"}]}}
```

**8. `submit_answer`** — correct answer, grades the card, advances the
schedule, `dueCount` drops to 0:

```json
>>> {"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"submit_answer","arguments":{"review_unit_id":"generated-quiz-deck-80cb605a305a472e-1-nato-letter-a","answer":"ALFA"}}}
<<< {"id":8,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"{\n  \"current\": {\n    \"grade\": {\"isCorrect\": true, \"rating\": 3, \"verdict\": \"correct\"},\n    \"scheduleChange\": {\"before\": null, \"after\": {\"reps\": 1, \"lapses\": 0, ...}}, ...\n  },\n  \"dueCount\": 0, ...\n}"}]}}
```

**9. `record_content_feedback`** — a kept/dropped verdict on the content
itself, distinct from grading the answer:

```json
>>> {"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"record_content_feedback","arguments":{"review_unit_id":"generated-quiz-deck-80cb605a305a472e-1-nato-letter-a","verdict":"kept","rationale":"clear and correct"}}}
<<< {"id":9,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"{\n  \"verdict\": \"kept\",\n  \"reviewUnitId\": \"generated-quiz-deck-80cb605a305a472e-1-nato-letter-a\", ...\n}"}]}}
```

**10. `invalidate_deck`** — retires the deck; due count stays at 0:

```json
>>> {"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"invalidate_deck","arguments":{"deck_id":"deck_80cb605a305a472e","event":"onboarding project closed"}}}
<<< {"id":10,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"{\n  \"current\": null,\n  \"drafts\": [],\n  \"dueCount\": 0, ...\n}"}]}}
```

**11. `list_due`** — stays retired:

```json
>>> {"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"list_due","arguments":{}}}
<<< {"id":11,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"{\n  \"dueCount\": 0,\n  \"nextPrompt\": null\n}"}]}}
```

This is the same eleven-call sequence `stdio_review_loop.rs` asserts
automatically; the run above is a hand-driven capture from a real terminal
against the queued generation-jobs path, not test output.

## Factory MCP registry entry

The canonical registry for bootstrapped-agent MCP visibility is
`~/.harness-kit/factory-mcps.yaml` (Harness Kit, `crates/harness-kit-checks/src/mcp_registry.rs`).
That file is out of scope for this ticket (a different repo, its own review
process), but the entry this server would need — matching the shape
`powder`'s entry already uses — is:

```yaml
- id: memory-engine
  app: Memory Engine
  source_repo: misty-step/memory-engine
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
