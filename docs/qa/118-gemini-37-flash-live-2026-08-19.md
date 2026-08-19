# 118 — Live Gemini 3.7 Flash generation receipt (2026-08-19)

Ticket: `scry-118-generation-receipt`. One live capture through the Scry
API that records `google/gemini-3.7-flash`. No bakeoff.

## Runtime

- Binary: `memory-engine-api` at `bcf6eaa` (`github/master` after `#131`)
- Store: local file store (`MEMORY_ENGINE_ENABLE_FILE_STORE=true`)
- Model env: `MEMORY_ENGINE_GENERATION_MODEL=google/gemini-3.7-flash`
  (same value as `DEFAULT_MODEL` and production `/etc/public-apps/scry.env`)
- Auth: service session for the dedicated dogfood account
- Host: `127.0.0.1:8818`

Production `public-apps` was `readyz` 503 (Postgres/generation worker
unavailable), so this receipt uses the same OpenRouter key and model
string the host already has configured, through the current API binary.

## Capture

Unstructured prose (no `Concept:` / `Question:` / `Answer:` blocks) about
the Antikythera mechanism. `POST /v1/service-sessions` →
`POST /v1/accounts/{id}/sources` →
`POST /v1/accounts/{id}/sources/{source_id}/generate`.

| Step | Route | Result | Time |
|---|---|---|---|
| Issue service session | `POST /v1/service-sessions` | 201 | 7 ms |
| Save source | `POST /v1/accounts/{id}/sources` | 201; `src_9cb8...` | 1 ms |
| Live generate | `POST /v1/accounts/{id}/sources/{id}/generate` | 200; 3 accepted drafts | 8 313 ms |
| Revoke session | `DELETE /v1/accounts/{id}/service-sessions/current` | 204 | 3 ms |

## Recorded generation run

`study-run-5e73398652c327fea3ba285dbb1d0d0b`

| Field | Value |
|---|---|
| provider | `openrouter` |
| model | `google/gemini-3.7-flash` |
| prompt version | `prompt-principled` |
| drafts | 3 accepted |
| validation failures | none |
| input tokens | 1 495 |
| output tokens | 979 |
| cost | 2 396 µUSD |
| provider latency | 3 191 ms |

Every draft's provenance names `openrouter` / `google/gemini-3.7-flash`.

A same-key OpenRouter chat completion from `public-apps` also returned
HTTP 200 with `"model":"google/gemini-3.7-flash"`.

## Cleanup

The service session was revoked. The OpenRouter key was not written into
this receipt or the repository. Production Postgres was not mutated.
