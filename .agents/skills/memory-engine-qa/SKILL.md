---
name: memory-engine-qa
description: |
  QA memory-engine changes by exercising the real running surface, not just tests.
  memory-engine is a Rust workspace: a framework-free learning kernel + facade
  library, an HTTP API/server-rendered study UI (memory-engine-api, deployed to
  DigitalOcean App Platform), model-backed generation, and dogfood clients.
  "Tests pass" is not QA.
  Use when: "QA this", "verify the feature", "smoke test", "check the app",
  "test memory-engine". Trigger: /memory-engine-qa.
argument-hint: "[api|kernel|ui|generation|gate|prod-smoke]"
---

# memory-engine-qa

QA in memory-engine means exercising the surface that changed against reality.
`bun run ci` is the deterministic gate (host Cargo: `cargo fmt --check`,
`cargo test --workspace`, `cargo clippy -D warnings`, `cargo doc`);
`bun run ci:full` adds the Dagger + Postgres + Gitleaks parity lane. Both are
**necessary but not sufficient**: they replay canned fixtures, so they prove the
learning *machinery* but say nothing about the model-backed **generation brain**
or the **live HTTP + study UI** actually serving requests. Those need real runs.

## Surfaces

| Changed area | Surface | QA path |
|---|---|---|
| `crates/memory-engine-core/**`, `crates/memory-engine/**` | Kernel + facade (library) | `cargo test -p memory-engine-core` / `-p memory-engine`; facade must compose without private-crate imports |
| `crates/memory-engine-api/**` | HTTP API + server-rendered study UI | Start local runtime; curl v1 JSON routes and walk the `/app/*` UI (below) |
| `crates/memory-engine-generation/**`, `-openrouter/**` | Generation brain | `cargo run -p memory-engine-bench -- generation`; live quality → dated receipts in `docs/evals/` — fixtures CANNOT prove this |
| `crates/memory-engine-web-shell/**`, `-cli`, `-import` | Dogfood clients | `cargo run -p memory-engine-web-shell -- --receipt` — confirm the JSON receipt, not just exit 0 |
| `-persistence*`, `-service`, `-study` | Boundary crates | `cargo test -p <crate>`; Postgres paths only run under `bun run ci:full` |

## Start local runtime (API + study UI)

`memory-engine-api` refuses to boot without BOTH a store and auth env. This
file-store + outbox combo is the local golden path (from README + `main.rs`):

```sh
MEMORY_ENGINE_ENABLE_FILE_STORE=true \
MEMORY_ENGINE_API_STORE_DIR=.tmp/api-dev \
MEMORY_ENGINE_AUTH_ALLOWED_EMAILS=owner@example.com \
MEMORY_ENGINE_AUTH_LINK_OUTBOX_PATH=.tmp/api-dev/outbox.tsv \
MEMORY_ENGINE_AUTH_EXPOSE_DEBUG_LINKS=true \
HOST=127.0.0.1 PORT=18080 \
cargo run -p memory-engine-api
# → "Memory Engine API listening on http://127.0.0.1:18080"
```

- Port: the production binary defaults to `8080`; use `18080` locally to avoid clashing.
- Auth/seed: `MEMORY_ENGINE_AUTH_EXPOSE_DEBUG_LINKS=true` surfaces the magic link
  on the "check your email" page so you can sign in without a mailer; or read the
  link from the outbox file `.tmp/api-dev/outbox.tsv`. Only the allowlisted email works.
- Model generation: set `OPENROUTER_API_KEY` (in `.env` — source it, never print/commit).
  Absent → generation silently falls back to structured-block parsing only.

## API QA

1. Health + home + anonymous-mutation boundary (mirrors the deploy smoke):
   ```sh
   curl -fsS http://127.0.0.1:18080/healthz            # {"status":"ok",...}
   curl -fsS -o /dev/null -w '%{http_code}\n' http://127.0.0.1:18080/          # 200
   curl -fsS -o /dev/null -w '%{http_code}\n' -X POST http://127.0.0.1:18080/app/generate  # expect 4xx
   ```
2. v1 JSON contract: `POST /v1/accounts` → `.../sources` → `.../sources/{id}/generate`
   → `POST /v1/accounts/{id}/review/next` (see `crates/memory-engine-api/src/routes.rs`;
   spec at `/v1/openapi.json`). Inspect response shape, not just status.
3. Study UI walk: sign in via the debug link, capture a source, generate a review,
   `/app/next` → reveal → submit. Confirm the review actually renders and grades.

## Generation QA (the brain — fixtures can't fake it)

For any generation/model change, run the fixture receipt, then read it — do not
trust exit 0. Green `bun run ci` says nothing about generation quality.

```sh
cargo run -p memory-engine-bench -- generation   # shape/variants/dup/bridge columns must stay green
```
Live model comparison is explicit and writes a dated receipt (needs
`OPENROUTER_API_KEY`; costs tokens — do not loop):
`cargo run -p memory-engine-bench -- generation --model <m> --judge <m> --out docs/evals/<name>-$(date +%F).md`.

## Deterministic gate + QA runner

```sh
bun run ci                                  # fast host gate (fmt, test, clippy, doc)
cargo run -p memory-engine-qa -- --local    # QA lane runner, inner loop
cargo run -p memory-engine-qa -- --full     # handoff sweep; ends with bun run ci:full (Dagger + Postgres + Gitleaks)
```

## Production smoke (optional)

Live at `https://memory-engine-api-i2xcr.ondigitalocean.app` on DigitalOcean
App Platform. Mirror the deploy smoke (health/home/anonymous mutation boundary)
per `docs/runbook.md`; e.g.
`curl -fsS https://memory-engine-api-i2xcr.ondigitalocean.app/healthz` →
`{"status":"ok",...}`.

## Gotchas

- **API won't boot** without a store (`MEMORY_ENGINE_POSTGRES_URL` OR the file-store trio)
  AND `MEMORY_ENGINE_AUTH_ALLOWED_EMAILS` + a mailer/outbox — it `exit(1)`s. #1 local trap.
- **File store is local/dev only** — production requires Neon Postgres; never use the file store in prod.
- **Generation falls back silently** without `OPENROUTER_API_KEY` — a "green" generate
  that never touched the model. `bun run ci` fixtures replay canned output.
- `.env` holds live secrets (RESEND, OPENROUTER). Source via env refs; never print or commit.
- `performance.benchmarks` / `bench -- generation` are receipt-only, not gating thresholds.
- Postgres/store contract tests only run under `bun run ci:full` (Dagger binds Postgres).

## Report

Return: **verdict** (PASS / FAIL / UNVERIFIED) · exact commands run · surfaces
exercised (machinery vs live API/UI vs generation brain) · artifacts inspected ·
what was NOT covered (e.g. "fixtures only — no live generation") and whether a
post-ship signal (DigitalOcean smoke, Canary) exists.
