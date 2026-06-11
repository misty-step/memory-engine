# Production runbook — memory-engine-api

Everything here is CLI/API-driven; no dashboards required. The app is
`memory-engine-api` on Fly (org `misty-step`, region `ord`), serving both the
JSON API and the server-rendered study UI from one Rust binary at
`https://memory-engine-api.fly.dev`.

## Deploy and rollback

Deploys are automatic and gated: pushing `master` runs the `ci` workflow;
`deploy.yml` runs only when ci succeeds for that SHA, deploys that exact SHA,
and fails on a post-deploy smoke regression (healthz, home page, anonymous
auth boundary).

Manual deploy (emergency only — bypasses the CI gate):

```sh
flyctl deploy --remote-only --app memory-engine-api
```

Rollback: redeploy the previous image.

```sh
flyctl releases --app memory-engine-api          # find prior image ref
flyctl deploy --app memory-engine-api --image <previous-image-ref>
```

## Health

```sh
curl -s https://memory-engine-api.fly.dev/healthz   # {"status":"ok",...}
flyctl status --app memory-engine-api
flyctl logs --app memory-engine-api
```

Errors land in Canary (`canary-obs` on Fly). Query it directly:

```sh
curl -s "$CANARY_ENDPOINT/api/v1/status" -H "Authorization: Bearer $CANARY_API_KEY"
curl -s "$CANARY_ENDPOINT/api/v1/report" -H "Authorization: Bearer $CANARY_API_KEY"
```

Every API 500 (`ApiFailure::internal`) is reported as service
`memory-engine-api`, environment `production`. Reporting is fire-and-forget;
losing Canary never affects requests.

## Secrets

Set via `flyctl secrets set --app memory-engine-api NAME=value`. Current set:

| Secret | Purpose |
| --- | --- |
| `MEMORY_ENGINE_POSTGRES_URL` | Neon pooled connection string (project `twilight-brook-49749008`, `memory-engine-prod`). |
| `MEMORY_ENGINE_AUTH_ALLOWED_EMAILS` | Comma-separated allowlist; account creation and magic links refuse other emails. |
| `MEMORY_ENGINE_AUTH_LINK_OUTBOX_PATH` | Magic-link outbox file (no email provider wired yet; see Login). |
| `OPENROUTER_API_KEY` | Enables model-backed generation for pasted prose; absent → structured-block parsing only. |
| `MEMORY_ENGINE_GENERATION_MODEL` | Optional model override (default `google/gemini-3.5-flash`; see docs/evals/). |
| `CANARY_ENDPOINT` / `CANARY_API_KEY` | Error reporting to Canary; absent → reporting is a no-op. |
| `MEMORY_ENGINE_ENVIRONMENT` | Environment label on Canary events. |

## Store backend selection

`main.rs` picks the backend at boot:

1. `MEMORY_ENGINE_POSTGRES_URL` set → Postgres (production).
2. Else `MEMORY_ENGINE_ENABLE_FILE_STORE=true` + `MEMORY_ENGINE_API_STORE_DIR`
   → file store (local/dev only; Fly machines have no volume, so file-store
   data dies on machine recycle — never use it in production).
3. Else: refuses to boot.

Migrations run once per process at first database use, not per request.

## Database (Neon)

Project `memory-engine-prod` (`twilight-brook-49749008`, aws-us-east-2),
fully managed by CLI:

```sh
neonctl projects list
neonctl connection-string --project-id twilight-brook-49749008 --pooled
neonctl branches create --project-id twilight-brook-49749008 --name migration-test
```

Use a branch to rehearse risky migrations against real data, then delete it.
The earlier Fly Managed Postgres cluster `memory-engine-api-pg` is superseded
by Neon and awaits operator decommission.

## Login (magic links over email)

Auth is allowlist + magic link, delivered by `bin/send-magic-link` (baked
into the image at `/usr/local/bin/send-magic-link`), which sends through
Resend from `onboarding@resend.dev` — deliverable only to the Resend account
owner's address, which matches the solo-dogfood allowlist. Activate with:

```sh
flyctl secrets set --app memory-engine-api \
  RESEND_API_KEY=<key> \
  MEMORY_ENGINE_AUTH_MAILER_COMMAND=/usr/local/bin/send-magic-link
```

While `MEMORY_ENGINE_AUTH_LINK_OUTBOX_PATH` is set instead, links land in
the outbox file on the machine:

```sh
flyctl ssh console --app memory-engine-api -C "tail -1 <outbox-path>"
```

A failed send surfaces as a 500 and therefore lands in Canary. After
verifying a domain in Resend, update the `from` address in
`bin/send-magic-link`.

`POST /app/account` is abuse-limited in the API boundary before any magic link
is sent. The fixed window is 5 attempts per 15 minutes per normalized email and
per client IP (`fly-client-ip`, then `x-real-ip`, then the first
`x-forwarded-for` value). Rejected requests return `429` with the generic
message "Too many sign-in attempts. Try again later."; they do not write an
outbox row or reveal whether an email is allowlisted.

The browser session is server-side. `POST /app/logout` requires the same CSRF
token as other app mutations, revokes the stored browser session, and clears
the `__Host-memory_engine_session` cookie with `Max-Age=0`. Reusing the old
cookie after logout must return `401`, including after a process restart.
