# MCP Review Loop Dogfood

Refs-Powder: memory-engine-071

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
speaks HTTP.

## Tool contract

Tools are agent-intent-shaped, not 1:1 REST wrappers — the discipline
`powder-mcp` established for this fleet (`../../Development/powder/crates/powder-mcp`
was the reference exemplar for the stdio transport, tool registration, and
error-handling shape).

| Tool | Composes | Intent |
|---|---|---|
| `create_deck` | `POST project-decks` -> `POST sources/{id}/generate` -> `POST drafts/{id}/keep` (per accepted draft) | Capture material as a study deck that is immediately due, not just a saved source record. |
| `list_decks` | `GET sources`, filtered to `projectKey.is_some()` | Check what decks exist, or find a `deck_id` to invalidate. |
| `invalidate_deck` | `POST project-decks/{id}/invalidate` | Retire every card from one deck after an external event (029's project-deck lifecycle). |
| `list_due` | `POST review/next` | Lightweight status check: how many are due, one-line teaser of the next prompt. |
| `review_next` | `POST review/next` | Full detail (prompt, choices, `review_unit_id`) to actually answer. |
| `submit_answer` | `POST review/{id}/submit` | Grade an answer and advance the schedule. |

`list_due` and `review_next` both call the same v1 route (`review/next` is
the only v1 route that returns due state — there is no separate read-only
"study view" route) but shape the response differently: `list_due` is a
peek, `review_next` is the full payload needed to work the card. Two
distinct agent intents over one endpoint, same discipline as
`next_app_review`/`app_study_view` in `memory-engine-api-state` calling
different backend methods for different UI intents.

## Credential model

Same env vars `docs/runbook.md` and `docs/dogfood/morning-review-cli.md`
already use: `MEMORY_ENGINE_ACCOUNT_ID` / `MEMORY_ENGINE_SESSION_TOKEN`. There
is no interactive `login` subcommand — stdin is the JSON-RPC channel, not a
terminal — so a brand-new local server instead bootstraps its own account
non-interactively from `MEMORY_ENGINE_MCP_EMAIL`, persisting the result to
`~/.memory-engine/mcp/credentials.json` (mode `0600`) for reuse across
restarts. `MEMORY_ENGINE_MCP_BASE_URL` overrides the base URL (default
`https://memory-engine-api-i2xcr.ondigitalocean.app`, matching
`memory-engine-review`).

Resolution order: env vars -> credentials file -> `MEMORY_ENGINE_MCP_EMAIL`
bootstrap -> fail loudly. There is no in-memory fallback
(`crates/memory-engine-mcp/tests/no_credentials_fallback.rs` proves the
server exits non-zero with no stdout before reading any stdin when none of
the three resolve) — the same lesson `powder-mcp` encoded after an
ephemeral in-memory mode silently evaporated claims on process exit.

## Commands

```sh
cargo build -p memory-engine-mcp
cargo test -p memory-engine-mcp
MEMORY_ENGINE_ACCOUNT_ID=... MEMORY_ENGINE_SESSION_TOKEN=... \
  MEMORY_ENGINE_MCP_BASE_URL=https://memory-engine-api-i2xcr.ondigitalocean.app \
  ./target/debug/memory-engine-mcp
```

## Falsifier

**Claim:** a cold agent (no prior context, no memory-engine-specific code)
can drive a full deck-create -> review -> invalidate loop purely over MCP
stdio JSON-RPC, using only the tool descriptions returned by `tools/list`.

**Would falsify it:** the loop cannot complete without reaching into
`memory-engine-api` internals, a tool silently drops server state between
calls, or a tool is a bare REST-route echo instead of an agent intent.

## Automated coverage

- `crates/memory-engine-mcp/src/lib.rs` unit tests: tool contract shape (six
  agent-intent tools, no REST-route names), schema validity, argument
  validation.
- `crates/memory-engine-mcp/src/session.rs` unit tests: credential
  resolution precedence and the no-fallback failure path.
- `crates/memory-engine-mcp/tests/no_credentials_fallback.rs`: spawns the
  real compiled binary with no credential path configured; asserts non-zero
  exit and no stdout.
- `crates/memory-engine-mcp/tests/stdio_review_loop.rs`: spawns the real
  compiled binary against a real local `memory-engine-api` (in-process axum
  server, not a mock) and drives the exact nine-call sequence below over its
  actual stdin/stdout pipes, asserting `dueCount` transitions 1 -> 0 and the
  deck stays retired after invalidation.

## Self-run transcript

A local `memory-engine-api` was started exactly per `docs/runbook.md`'s
local file-store pattern (the same Rust binary that runs in production,
pointed at a scratch store dir):

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

**2. `tools/list`** — six agent-intent tools (truncated here; full output is
identical to the table above rendered as JSON Schema):

```json
>>> {"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}
<<< {"id":2,"jsonrpc":"2.0","result":{"tools":[{"name":"create_deck",...},{"name":"list_decks",...},{"name":"invalidate_deck",...},{"name":"list_due",...},{"name":"review_next",...},{"name":"submit_answer",...}]}}
```

**3. `create_deck`** — one call saves the source, generates a quiz card, and
keeps it explicitly. `keptCardCount: 1` proves the composition, not a bare save:

```json
>>> {"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"create_deck","arguments":{"project_key":"nato-onboarding","title":"NATO letter A fixture","body":"Concept: NATO letter A\nActivity: quiz\nStage: recognition-3\nQuestion: What is the NATO phonetic alphabet word for A?\nAnswer: ALFA\nDistractors: BRAVO, CHARLIE\nReference: The NATO phonetic alphabet word for A is ALFA."}}}
<<< {"id":3,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"{\n  \"keptCardCount\": 1,\n  \"deck\": {\n    \"deckId\": \"deck_0d32f2a69298d5d8\",\n    \"projectKey\": \"nato-onboarding\",\n    \"source\": {\"sourceId\": \"deck_0d32f2a69298d5d8\", \"title\": \"NATO letter A fixture\", ...}\n  }\n}"}]}}
```

**4. `list_decks`** — the new deck is visible, scoped to its `project_key`:

```json
>>> {"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"list_decks","arguments":{"project_key":"nato-onboarding"}}}
<<< {"id":4,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"[{\"sourceId\": \"deck_0d32f2a69298d5d8\", \"projectKey\": \"nato-onboarding\", \"title\": \"NATO letter A fixture\", ...}]"}]}}
```

**5. `list_due`** — a lightweight peek, one due card:

```json
>>> {"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"list_due","arguments":{}}}
<<< {"id":5,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"{\n  \"dueCount\": 1,\n  \"nextPrompt\": \"In the NATO phonetic alphabet, which code word represents the letter A?\"\n}"}]}}
```

**6. `review_next`** — full detail, including `reviewUnitId` needed to
answer:

```json
>>> {"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"review_next","arguments":{}}}
<<< {"id":6,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"{\n  \"current\": {\n    \"choices\": [\"ARCHER\", \"APEX\", \"ALFA\", \"AMBER\"],\n    \"prompt\": \"In the NATO phonetic alphabet, which code word represents the letter A?\",\n    \"reviewUnitId\": \"generated-quiz-deck-0d32f2a69298d5d8-1-nato-alphabet-a\"\n  },\n  \"dueCount\": 1\n}"}]}}
```

**7. `submit_answer`** — correct answer, `dueCount` drops to 0:

```json
>>> {"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"submit_answer","arguments":{"review_unit_id":"generated-quiz-deck-0d32f2a69298d5d8-1-nato-alphabet-a","answer":"ALFA"}}}
<<< {"id":7,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"{\n  \"current\": {\n    \"grade\": {\"isCorrect\": true, \"rating\": 3, \"verdict\": \"correct\"},\n    \"expectedAnswer\": \"ALFA\"\n  },\n  \"dueCount\": 0\n}"}]}}
```

**8. `invalidate_deck`** — retires the deck:

```json
>>> {"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"invalidate_deck","arguments":{"deck_id":"deck_0d32f2a69298d5d8","event":"onboarding project closed"}}}
<<< {"id":8,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"{\n  \"current\": null,\n  \"drafts\": [],\n  \"dueCount\": 0\n}"}]}}
```

**9. `list_due`** — stays retired:

```json
>>> {"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"list_due","arguments":{}}}
<<< {"id":9,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"{\n  \"dueCount\": 0,\n  \"nextPrompt\": null\n}"}]}}
```

This is the same nine-call sequence `stdio_review_loop.rs` asserts
automatically; the run above is the hand-driven capture from a real
terminal, not test output.

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
