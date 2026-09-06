---
name: scry-qa
description: >
  Exercise the changed Scry surface against reality: kernel, API/UI,
  generation, dogfood clients, or production smoke. Use for QA, verification,
  smoke tests, or checking the app.
argument-hint: "[api|kernel|ui|generation|gate|prod-smoke]"
---

# Scry QA

Choose the surface that changed. Scry's production runtime is the
native Rust `memory-engine-api` process on the isolated public application host.
A green fixture or build proves only the machinery it exercises; live API/UI
and model-backed generation need their own runs.

| Changed area | Surface and proof |
|---|---|
| `crates/memory-engine-core/**`, `crates/memory-engine/**` | `cargo test -p memory-engine-core` / `-p memory-engine`; facade composes without private-crate imports |
| `crates/memory-engine-api/**` | Run the API, exercise v1 JSON routes and `/app/*` UI |
| `crates/memory-engine-generation/**`, `-openrouter/**` | `cargo run -p memory-engine-bench -- generation`; live quality needs a dated `docs/evals/` receipt |
| `crates/memory-engine-web-shell/**`, `-cli`, `-import` | `cargo run -p memory-engine-web-shell -- --receipt`; inspect the JSON receipt |
| persistence, service, study crates | Targeted crate tests; Postgres paths run under `bun run ci:full` |

## Local API

The native API needs a store, an allowlisted auth email, an outbox/mailer, and
`MEMORY_ENGINE_RETURN_UNSUBSCRIBE_SECRET`. The file-store path is local/dev
only:

```sh
MEMORY_ENGINE_ENABLE_FILE_STORE=true \
MEMORY_ENGINE_API_STORE_DIR=.tmp/api-dev \
MEMORY_ENGINE_AUTH_ALLOWED_EMAILS=owner@example.com \
MEMORY_ENGINE_AUTH_LINK_OUTBOX_PATH=.tmp/api-dev/outbox.tsv \
MEMORY_ENGINE_AUTH_EXPOSE_DEBUG_LINKS=true \
MEMORY_ENGINE_RETURN_UNSUBSCRIBE_SECRET=local-dev-unsubscribe-secret \
HOST=127.0.0.1 PORT=18080 cargo run -p memory-engine-api
```

With the process running, check health, home, and the anonymous mutation
boundary:

```sh
curl -fsS http://127.0.0.1:18080/healthz
curl -fsS -o /dev/null -w '%{http_code}\n' http://127.0.0.1:18080/
curl -fsS -o /dev/null -w '%{http_code}\n' -X POST http://127.0.0.1:18080/app/generate
```

Then exercise `POST /v1/accounts`, source capture, queued
`POST .../generation-jobs`, bounded polling of
`GET .../generation-jobs/{jobId}`, and review-next. Walk sign-in via the debug
link, source capture, generation, `/app/next`, reveal, and submit. The legacy
synchronous generate route returns HTTP 409 when Postgres is configured.

Generation without `OPENROUTER_API_KEY` silently uses structured-block parsing;
source the key from `.env` without printing or committing it. The fixture
receipt cannot prove model quality.

## Gates and production

```sh
bun run ci
bun run ci:full
cargo run -p memory-engine-qa -- --local
cargo run -p memory-engine-qa -- --full
```

For a live model comparison, write one dated receipt and do not loop:

```sh
cargo run -p memory-engine-bench -- generation --model <m> --judge <m> --out docs/evals/<name>-$(date +%F).md
```

Production is `https://scry.study` on the native `scry.service` process with
Neon Postgres. Use that branded origin for smoke checks (for example,
`curl -fsS https://scry.study/readyz`); there is no provider-origin fallback.
Never use the file store in production. Postgres contract tests run under
`bun run ci:full`.

## Report

Return `PASS`, `FAIL`, or `UNVERIFIED`; exact commands; surfaces exercised
(machinery, live API/UI, generation brain); artifacts inspected; uncovered
surfaces; and any public-host smoke or Canary signal.
