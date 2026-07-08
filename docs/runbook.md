# Production runbook — memory-engine-api

Everything here is CLI/API-driven; no dashboards required. The app is
`memory-engine-api` on DigitalOcean App Platform (cut over from Fly
2026-07-08; Postgres is Neon, unchanged by the cutover), serving both the
JSON API and the server-rendered study UI from one Rust binary at
`https://memory-engine-api-i2xcr.ondigitalocean.app`. The Fly app
(`memory-engine-api.fly.dev`, org `misty-step`, region `ord`) is left
running pending decommission; do not point new consumers at it.

## Agent surface summary

- App: `memory-engine-api`.
- Primary platform: DigitalOcean App Platform (id
  `5ab05b73-9265-43c9-a01c-fef53f5f46a4`), URL
  `https://memory-engine-api-i2xcr.ondigitalocean.app`.
- Standby platform: Fly Machines, org `misty-step`, primary region `ord`,
  URL `https://memory-engine-api.fly.dev` — still deployed by CI on every
  push (see Deployed smoke below) and kept warm pending decommission, but
  no longer where consumers should point.
- Runtime: Rust binary from `crates/memory-engine-api`, built by `Dockerfile`.
- Store contract: production must set `MEMORY_ENGINE_POSTGRES_URL`; file store
  requires `MEMORY_ENGINE_ENABLE_FILE_STORE=true` and is local/dev only.
- Auth contract: allowlist plus magic links; production account creation and
  magic-link delivery require `MEMORY_ENGINE_AUTH_ALLOWED_EMAILS` plus either
  `MEMORY_ENGINE_AUTH_MAILER_COMMAND` or the temporary outbox path.
- Smoke contract: deploys run health, home-page, and anonymous mutation
  boundary checks from `.github/workflows/deploy.yml`; agents can repeat the
  exact commands below.

## Deployed smoke

These commands mirror the post-deploy smoke in `.github/workflows/deploy.yml`,
which still targets the Fly standby app. For the primary DO app, rerun the
same three checks against `https://memory-engine-api-i2xcr.ondigitalocean.app`.

```sh
base="https://memory-engine-api.fly.dev"

status=$(curl -fsS --max-time 15 -o /tmp/memory-engine-healthz -w "%{http_code}" "$base/healthz")
test "$status" = "200"
grep -q '"status":"ok"' /tmp/memory-engine-healthz

status=$(curl -fsS --max-time 15 -o /tmp/memory-engine-home -w "%{http_code}" "$base/")
test "$status" = "200"

status=$(curl -fsS --max-time 15 -o /tmp/memory-engine-auth-boundary -w "%{http_code}" -X POST "$base/app/generate")
case "$status" in 4??) ;; *) echo "expected 4xx, got $status"; exit 1;; esac
```

## Production generation latency

Use this when a ticket needs model-backed production latency evidence. It
uses an existing allowlisted v1 account/session, saves one article-sized
source, times the end-to-end `generate` request, records the response, and
archives the source. Production account creation is allowlist-protected, so do
not use a throwaway email here; export a pre-provisioned account id and session
token first.

```sh
set -euo pipefail

base="https://memory-engine-api-i2xcr.ondigitalocean.app"
receipt_dir="docs/qa"
stamp="$(date -u +%Y%m%dT%H%M%SZ)"
account_id="${MEMORY_ENGINE_ACCOUNT_ID:?set MEMORY_ENGINE_ACCOUNT_ID}"
session_token="${MEMORY_ENGINE_SESSION_TOKEN:?set MEMORY_ENGINE_SESSION_TOKEN}"
source_id=""
cleanup_source() {
  if [ -n "$source_id" ]; then
    curl -fsS --max-time 20 \
      -H "authorization: Bearer $session_token" \
      -X DELETE \
      "$base/v1/accounts/$account_id/sources/$source_id" \
      >/dev/null || true
  fi
}
trap cleanup_source EXIT
body='Spaced practice improves long-term retention because each retrieval attempt forces the learner to reconstruct the memory rather than reread it passively. Feedback closes the loop by showing whether the reconstruction was accurate. When practice is delayed, the extra effort strengthens later access, but if the delay is too long the learner may fail without a useful cue. A good study system therefore mixes short recognition checks, cued recall, free recall, and applied composition so the learner moves from identifying an answer toward using the idea in context.'

source_json="$(jq -n --arg title "Latency receipt $stamp" --arg body "$body" \
  '{title:$title, body:$body}')"
source_response="$(curl -fsS --max-time 20 \
  -H 'content-type: application/json' \
  -H "authorization: Bearer $session_token" \
  -d "$source_json" \
  "$base/v1/accounts/$account_id/sources")"
source_id="$(printf '%s' "$source_response" | jq -r '.sourceId')"

generate_status="$(curl -fsS --max-time 150 \
  -H "authorization: Bearer $session_token" \
  -o "$receipt_dir/production-generation-$stamp.json" \
  -w "status=%{http_code} time_total=%{time_total}\n" \
  -X POST \
  "$base/v1/accounts/$account_id/sources/$source_id/generate")"
case "$generate_status" in
  status=2??\ *) ;;
  *) echo "generation failed: $generate_status"; exit 1 ;;
esac
printf '%s\n' "$generate_status" \
  | tee "$receipt_dir/production-generation-$stamp.latency.txt"
```

## Deploy and rollback

DigitalOcean App Platform is git-integrated on `master`: every push builds
and deploys automatically (`doctl apps list-deployments memory-engine-api`
to watch). The legacy Fly `deploy.yml` workflow still runs in parallel
(gated on the `ci` workflow, deploys the same SHA, fails on a post-deploy
smoke regression) and keeps the standby Fly app current until it is
decommissioned — do not remove it without an explicit decommission ticket.

Manual DO deploy / rollback:

```sh
doctl apps create-deployment 5ab05b73-9265-43c9-a01c-fef53f5f46a4   # redeploy current spec
doctl apps list-deployments 5ab05b73-9265-43c9-a01c-fef53f5f46a4    # find a prior deployment id to roll back to
```

Manual Fly deploy (standby app, emergency only):

```sh
flyctl deploy --remote-only --app memory-engine-api
```

## Health

```sh
curl -s https://memory-engine-api-i2xcr.ondigitalocean.app/healthz   # {"status":"ok",...}
doctl apps get 5ab05b73-9265-43c9-a01c-fef53f5f46a4

# Fly standby (not primary traffic):
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

A failed send surfaces as a 500 and therefore lands in Canary. The sender
address is the `MEMORY_ENGINE_MAIL_FROM` secret (default
`Memory Engine <onboarding@resend.dev>`); switching it is a secret change, not
a code edit — see Deliverability below.

### Deliverability (inbox, not spam)

`onboarding@resend.dev` is a shared sender with no domain reputation, so Gmail
spam-folders it — expected, not a script bug. To reach the inbox, send from a
verified domain. Resend verification is fully API-driven (no dashboard):

1. **Choose a domain the operator controls** — a subdomain such as
   `mail.<domain>` keeps the apex's reputation separate. *Operator decision.*
2. **Register it** (returns the DNS records to add):
   ```sh
   curl -s -X POST https://api.resend.com/domains \
     -H "Authorization: Bearer $RESEND_API_KEY" -H "Content-Type: application/json" \
     -d '{"name":"mail.<domain>"}'
   # response.records[] = the SPF (TXT), DKIM (TXT/CNAME), and DMARC (TXT) records.
   ```
3. **Add those DNS records** at the domain's host. *Operator / DNS-token-gated.*
4. **Verify + confirm propagation:**
   ```sh
   curl -s -X POST https://api.resend.com/domains/<id>/verify -H "Authorization: Bearer $RESEND_API_KEY"
   dig +short TXT <selector>._domainkey.mail.<domain>   # DKIM (selector from the step-2 records[])
   dig +short TXT mail.<domain>                          # SPF (v=spf1 … include:amazonses.com)
   dig +short TXT _dmarc.mail.<domain>                   # DMARC (v=DMARC1; p=none …)
   ```
5. **Switch the sender** (no redeploy of the script):
   ```sh
   flyctl secrets set --app memory-engine-api MEMORY_ENGINE_MAIL_FROM="Memory Engine <noreply@mail.<domain>>"
   ```
6. **Confirm inbox placement** — trigger a fresh magic link to an allowlist
   address; verify it lands in the Gmail inbox, not spam. *Operator-confirmed.*

Content is already tuned for deliverability: real from-name, plain-text body, the
full sign-in URL (no shortener), a plain subject. Keep it that way.

**"Didn't get the email" — first diagnostic:** check Resend's delivery status for
the send, then Canary for a 500 on the send path:
```sh
curl -s https://api.resend.com/emails/<email-id> -H "Authorization: Bearer $RESEND_API_KEY"
```

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
