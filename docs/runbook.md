# Production runbook — memory-engine-api

Everything here is CLI/API-driven; no dashboards required. The app is
`memory-engine-api` on DigitalOcean App Platform, with Postgres on Neon,
serving both the JSON API and the server-rendered study UI from one Rust binary
at `https://memory-engine-api-i2xcr.ondigitalocean.app`. DigitalOcean is the
only current application runtime; rollback stays within DigitalOcean plus git.

## Agent surface summary

- App: `memory-engine-api`.
- Platform: DigitalOcean App Platform (id
  `5ab05b73-9265-43c9-a01c-fef53f5f46a4`), URL
  `https://memory-engine-api-i2xcr.ondigitalocean.app`.
- Runtime: Rust binary from `crates/memory-engine-api`, built by `Dockerfile`.
- Store contract: production must set `MEMORY_ENGINE_POSTGRES_URL`; file store
  requires `MEMORY_ENGINE_ENABLE_FILE_STORE=true` and is local/dev only.
- Auth contract: allowlist plus magic links; production account creation and
  magic-link delivery require `MEMORY_ENGINE_AUTH_ALLOWED_EMAILS` plus either
  `MEMORY_ENGINE_AUTH_MAILER_COMMAND` or the temporary outbox path, and
  `MEMORY_ENGINE_RETURN_UNSUBSCRIBE_SECRET` for signed reminder links.
- Return scheduler: the API process owns a bounded, Postgres-backed sweep;
  disable it with `MEMORY_ENGINE_RETURN_NOTIFICATION_SCHEDULER_ENABLED=false`,
  and keep the operator-only `MEMORY_ENGINE_RETURN_NOTIFICATION_MANUAL_TOKEN`
  out of source control.
- Smoke contract: every DigitalOcean deployment runs the health, home-page,
  and anonymous mutation boundary checks below before it can be called live.

## Deployed smoke

These commands exercise the sole production runtime. The retired provider
workflow must not be restored: it previously reactivated an obsolete runtime
after every green `master` push.

```sh
base="https://memory-engine-api-i2xcr.ondigitalocean.app"

status=$(curl -fsS --max-time 15 -o /tmp/memory-engine-healthz -w "%{http_code}" "$base/healthz")
test "$status" = "200"
grep -q '"status":"ok"' /tmp/memory-engine-healthz

status=$(curl -fsS --max-time 15 -o /tmp/memory-engine-home -w "%{http_code}" "$base/")
test "$status" = "200"

status=$(curl -fsS --max-time 15 -o /tmp/memory-engine-auth-boundary -w "%{http_code}" -X POST "$base/app/generate")
case "$status" in 4??) ;; *) echo "expected 4xx, got $status"; exit 1;; esac

curl -fsS --max-time 15 "$base/manifest.webmanifest" | jq -e \
  '.display == "standalone" and (.icons | length >= 2)' >/dev/null
curl -fsS --max-time 15 "$base/favicon.png" | file - | grep -q 'PNG image data, 192 x 192'
curl -fsS --max-time 15 "$base/apple-touch-icon.png" | file - | grep -q 'PNG image data, 180 x 180'
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

DigitalOcean App Platform is git-integrated on `master`, but the app spec
does NOT set `deploy_on_push`, so **merging to master does not deploy** —
verified 2026-07-09 when two shipped merges sat undeployed until a manual
trigger. Until ticket 075 wires an automatic gated deploy, every ship must
end with the manual deploy below plus the Deployed smoke, and "shipped"
claims must name the ACTIVE deployment id. The 2026-07-08 cutover removed the
legacy provider workflow after it was proven to reactivate the old runtime on
every green push.

Manual deploy:

```sh
app_id="5ab05b73-9265-43c9-a01c-fef53f5f46a4"
doctl apps create-deployment "$app_id" --wait
doctl apps list-deployments "$app_id" --format ID,Phase,Created,Updated
```

Code rollback is a normal git revert followed by a new DigitalOcean deployment;
do not revive a second provider. For an app-spec rollback, retrieve the known
good deployment's spec, validate it, update the same app, and rerun the smoke:

```sh
app_id="5ab05b73-9265-43c9-a01c-fef53f5f46a4"
known_good_deployment="${KNOWN_GOOD_DEPLOYMENT_ID:?set deployment id}"
umask 077
doctl apps spec get "$app_id" --deployment "$known_good_deployment" \
  > /tmp/memory-engine-known-good.yaml
doctl apps spec validate /tmp/memory-engine-known-good.yaml
doctl apps update "$app_id" --spec /tmp/memory-engine-known-good.yaml --wait
rm -f /tmp/memory-engine-known-good.yaml
```

## Health

```sh
curl -s https://memory-engine-api-i2xcr.ondigitalocean.app/healthz   # {"status":"ok",...}
doctl apps get 5ab05b73-9265-43c9-a01c-fef53f5f46a4
```

Errors land in Canary. Query it directly:

```sh
curl -s "$CANARY_ENDPOINT/api/v1/status" -H "Authorization: Bearer $CANARY_API_KEY"
curl -s "$CANARY_ENDPOINT/api/v1/report" -H "Authorization: Bearer $CANARY_API_KEY"
```

Every API 500 (`ApiFailure::internal`) is reported as service
`memory-engine-api`, environment `production`. Reporting is fire-and-forget;
losing Canary never affects requests.

## Secrets

The DigitalOcean app owns the only live values below. Keep every secret typed as
an encrypted App Platform variable and never copy its value into this repo.

| Secret | Purpose |
| --- | --- |
| `MEMORY_ENGINE_POSTGRES_URL` | Neon pooled connection string (project `twilight-brook-49749008`, `memory-engine-prod`). |
| `MEMORY_ENGINE_AUTH_ALLOWED_EMAILS` | Comma-separated allowlist; account creation and magic links refuse other emails. |
| `MEMORY_ENGINE_AUTH_MAILER_COMMAND` | Production command path `/usr/local/bin/send-magic-link`; keep encrypted and do not replace with the local outbox. |
| `RESEND_API_KEY` | Encrypted Resend credential consumed only by the bundled mailer; never print or place in a command line. |
| `MEMORY_ENGINE_MAIL_FROM` | Replyable production sender on the verified `mistystep.io` domain; keep the operator-approved sender and do not switch domains without a new ruling. |
| `MEMORY_ENGINE_PUBLIC_BASE_URL` | Public link base used to construct the sign-in URL; keep aligned with the deployed Scry host. |
| `MEMORY_ENGINE_RETURN_UNSUBSCRIBE_SECRET` | Stable secret for HMAC-signed, seven-day, account/email-scoped reminder unsubscribe links. |
| `MEMORY_ENGINE_RETURN_NOTIFICATION_SCHEDULER_ENABLED` | Optional kill switch; set `false` to disable scheduled reminder sweeps. |
| `MEMORY_ENGINE_RETURN_NOTIFICATION_MANUAL_TOKEN` | Operator-only token for bounded manual scheduler runs; store encrypted. |
| `MEMORY_ENGINE_RETURN_NOTIFICATION_BATCH_SIZE` | Optional bounded sweep size, default 100 and capped at 1000. |
| `MEMORY_ENGINE_RETURN_NOTIFICATION_SCHEDULER_INTERVAL_SECONDS` | Optional sweep interval, default 900 seconds and capped at 86400. |
| `MEMORY_ENGINE_AUTH_LINK_OUTBOX_PATH` | Local/dev-only magic-link outbox fallback; never use it as production delivery proof. |
| `OPENROUTER_API_KEY` | Enables model-backed generation for pasted prose; absent → structured-block parsing only. |
| `MEMORY_ENGINE_GENERATION_MODEL` | Optional model override (default `google/gemini-3.5-flash`; see docs/evals/). |
| `CANARY_ENDPOINT` / `CANARY_API_KEY` | Error reporting to Canary; absent → reporting is a no-op. |
| `MEMORY_ENGINE_ENVIRONMENT` | Environment label on Canary events. |
## Store backend selection

`main.rs` picks the backend at boot:

1. `MEMORY_ENGINE_POSTGRES_URL` set → Postgres (production).
2. Else `MEMORY_ENGINE_ENABLE_FILE_STORE=true` + `MEMORY_ENGINE_API_STORE_DIR`
   → file store (local/dev only; it is not durable production storage).
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

## Login (magic links over email)

Auth is allowlist + magic link, delivered by `bin/send-magic-link` (baked
into the image at `/usr/local/bin/send-magic-link`). Production sends through
Resend using the encrypted `RESEND_API_KEY` and a replyable
`MEMORY_ENGINE_MAIL_FROM` address on the verified `mistystep.io` domain. The
operator ruling is to keep that verified domain and sender, without upgrading
billing, adding `scry.study`, or deleting `mistystep.io`. Keep
`MEMORY_ENGINE_AUTH_MAILER_COMMAND=/usr/local/bin/send-magic-link` encrypted in
the DigitalOcean app spec and never put the key in a shell command or checked-in
spec.

The bundled sender has two environment contracts. Magic-link mode uses
`MEMORY_ENGINE_AUTH_EMAIL` and `MEMORY_ENGINE_AUTH_LINK` and does not require
an idempotency key. Due-count reminder mode fails closed unless
`MEMORY_ENGINE_RETURN_NOTIFICATION_IDEMPOTENCY_KEY` is present; it sends that
same value as Resend's `Idempotency-Key` HTTP header. Retries of one durable
reminder claim must reuse both the key and the original payload so Resend can
deduplicate the `POST /emails` request. The key is not shared with magic-link
mail.

While `MEMORY_ENGINE_AUTH_LINK_OUTBOX_PATH` is set instead, links land in an
instance-local outbox. That path is a temporary solo-dogfood fallback, not a
durable delivery channel.

A failed send surfaces as a 500 and therefore lands in Canary. The production
sender is the encrypted `MEMORY_ENGINE_MAIL_FROM` value on verified
`mistystep.io`; the script fallback `onboarding@resend.dev` is local-only and must
not be used as production proof. Switching the sender is a secret change, not a
code edit — see Deliverability below.

### Due-count return channel

The signed-in workspace offers one explicit, optional return channel: “Enable
due-count reminders.” It stores the normalized, allowlisted reminder address in
the account store and sends one confirmation through the same
`MEMORY_ENGINE_AUTH_MAILER_COMMAND` / `MEMORY_ENGINE_AUTH_LINK_OUTBOX_PATH`
boundary. The scheduled boundary enumerates enabled preferences without a
browser request, computes each account's live due count, and sends only when
reviews are due and the persisted last-send time is at least 24 hours old. The
policy is deterministic, has no streaks or promotional content, and a learner
can disable it from the workspace or the signed unsubscribe link in the
plain-text message. The email GET only renders a confirmation; its POST carries
the scoped token and performs the mutation without requiring a browser session.
Disable is persisted and never sends mail. Home/render GETs are read-only and
never invoke this boundary.

### Scheduled execution and operations

Every API instance starts the scheduler unless
`MEMORY_ENGINE_RETURN_NOTIFICATION_SCHEDULER_ENABLED=false`. Each sweep is
bounded by `MEMORY_ENGINE_RETURN_NOTIFICATION_BATCH_SIZE` (default 100,
maximum 1000), runs at
`MEMORY_ENGINE_RETURN_NOTIFICATION_SCHEDULER_INTERVAL_SECONDS` (default 900),
and uses the durable per-account claim as the multi-instance lease/fence. The
current implementation deliberately uses one synchronous worker per instance;
Postgres/file claims provide the cross-instance concurrency bound and provider
idempotency prevents duplicate logical sends.

The file adapter's per-account notification lock is a persistent path with an
OS descriptor lock acquired nonblockingly. It is never deleted as part of
ownership release, so stale paths are harmless; a process crash releases the
descriptor and a contending scheduler skips that account until it can acquire
the lock. The same libc-backed helper protects the repository-owned file
outbox: it scans durable delivery keys while holding the descriptor lock and
does not append a duplicate after a lease-expiry reclaim.

The scheduler returns an owned lifecycle handle. The production binary joins
that handle during graceful shutdown, including any in-flight blocking
provider call, before exiting. Manual and interactive reminder routes run
storage and provider work on blocking workers; health requests remain
responsive while a provider is slow. `lastRunAtMs` records every sweep, while
`lastSuccessAtMs` advances only for a sweep with zero failed accounts.

The liveness counters are included in `/healthz` under
`returnNotificationScheduler`. A bounded manual/backfill run is available only
with the operator token:

```sh
curl -fsS -X POST \
  -H "x-scheduler-token: ${MEMORY_ENGINE_RETURN_NOTIFICATION_MANUAL_TOKEN:?set token}" \
  "$base/internal/scheduler/return-notifications"
```

A failed provider send releases the claim but preserves the complete 092
delivery envelope and applies bounded exponential retry backoff (one minute,
doubling to six hours). A crash after provider acceptance retries only after
the lease expires with the same idempotency key and payload; stale finalize is
fenced by claim id. To disable or roll back the trigger, set the scheduler
flag false or revert the application commit and deploy through the normal
DigitalOcean rollback procedure below. Inspect `/healthz`, provider send logs,
and Canary failures together during an incident.

Each unsubscribe link is a seven-day HMAC token bound to the account, normalized
email, and a persisted unsubscribe nonce. The GET remains read-only; the POST
atomically compares the current nonce and rotates it while disabling the
preference, so a replayed or concurrent stale bearer cannot win against an
authenticated re-enable. The nonce column is an additive migration with an
empty default for existing Postgres rows, and legacy file JSON defaults the
same way. Legacy v1 links are intentionally rejected because they cannot carry
the nonce; the next authenticated enable or reminder delivery backfills the
nonce and issues only v2 links.

The command boundary receives these variables for a due-count message:
`MEMORY_ENGINE_RETURN_NOTIFICATION_EMAIL`,
`MEMORY_ENGINE_RETURN_NOTIFICATION_DUE_COUNT`, and
`MEMORY_ENGINE_RETURN_NOTIFICATION_UNSUBSCRIBE`; it also receives
`MEMORY_ENGINE_RETURN_NOTIFICATION_IDEMPOTENCY_KEY` so a retry can reuse the
same durable delivery identity. The bundled sender requires that key in
reminder mode and forwards it using Resend's supported `Idempotency-Key`
contract; a missing key is an error before any provider request. The bundled
sender supports both the magic-link and due-count envelopes. A file outbox line
beginning with `due-count` is a local proof receipt; production proof still
requires checking the provider send log and inbox placement.

### Deliverability (verified mistystep.io sender)

The operator elected to keep the existing verified `mistystep.io` Resend domain
and sender. Do not upgrade Resend billing, add `scry.study`, or delete
`mistystep.io` as part of this card. The public product/link host remains
`scry.study`; the resulting sender/link-domain mismatch is an explicit,
operator-approved residual risk, not a reason to silently change the sender.

Verify the active provider and DNS state without exposing credentials or message
content:

```sh
# Resend: use the deployed scoped Mint alias secret://memory-engine/resend-domain
# or an authenticated Resend operator surface. Never print the credential.
# The route allows only GET /domains and GET /domains/<id>; expected: mistystep.io
# remains status=verified.
dig +short TXT send.mistystep.io
dig +short TXT resend._domainkey.mistystep.io
dig +short TXT _dmarc.mistystep.io
```

The bundled mailer emits only one bounded diagnostic line to stderr/application
logs after a successful provider call:
`resend_status=accepted resend_id=<provider-id>`. Transport failures emit
`resend_status=transport_error`; non-2xx provider responses emit only
`resend_status=failed http_status=<status>`. No recipient, token, full link,
request body, or provider response body is logged. Control characters in recipient,
sender, subject, and reminder idempotency inputs fail closed before any provider
request; message paragraph breaks are encoded once as JSON newlines. Keep provider
IDs as bounded operator evidence and redact them in shared proof when not needed
for lookup.

For a delivery investigation, use the provider operator surface with the
redacted provider ID and classify the send as `accepted`, `delivered`,
`bounced`, `complained`, `delayed`, or `unknown`. Do not copy message
content or recipient addresses into logs. A provider-accepted event alone does
not prove Inbox placement.

### Production magic-link proof and rollback

1. Trigger one fresh sign-in request through the normal production Scry UI for
the already-approved invited Gmail account. Use a unique nonce in the test
request metadata only; never paste the nonce, recipient, token, or full link into
proof.
2. In Gmail, record only the classification (Inbox or Spam) and privacy-safe
source-header results for SPF, DKIM, and DMARC. Do not screenshot message body,
recipient, token, or the full link.
3. Open the link once in the production Scry UI and record successful sign-in.
Attempt the same link again and record rejection. Attempt the link from a second
account/session and record rejection; do not record either account identity.
4. If placement or authentication fails, restore the previous known-good
`MEMORY_ENGINE_MAIL_FROM` value on the verified `mistystep.io` domain,
deploy through DigitalOcean, and rerun the deployed smoke. Do not delete the
verified domain. The temporary outbox remains a local-only fallback and must not
be enabled as a production delivery claim.

Residual risk: Gmail placement can vary because the approved sender domain and
public link host differ. Re-run the bounded Inbox/Spam and source-header proof
after any sender, DNS, or provider reputation change.

`POST /app/account` is abuse-limited in the API boundary before any magic link
is sent. The fixed window is 5 attempts per 15 minutes per normalized email and
per client IP (`do-connecting-ip`, then `x-real-ip`, then the first
`x-forwarded-for` value). Rejected requests return `429` with the generic
message "Too many sign-in attempts. Try again later."; they do not write an
outbox row or reveal whether an email is allowlisted.

The browser session is server-side. `POST /app/logout` requires the same CSRF
token as other app mutations, revokes the stored browser session, and clears
the `__Host-memory_engine_session` cookie with `Max-Age=0`. Reusing the old
cookie after logout must return `401`, including after a process restart.
