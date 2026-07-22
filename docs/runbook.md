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
  Machine consumers use operator-gated service sessions
  (`MEMORY_ENGINE_ADMIN_TOKEN`, below) — never email.
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

# Readiness is dependency-aware: it is only 200 when Postgres and the
# generation worker are available. `/healthz` above remains liveness-only.
status=$(curl -fsS --max-time 15 -o /tmp/memory-engine-readyz -w "%{http_code}" "$base/readyz")
test "$status" = "200"
grep -q '"status":"ready"' /tmp/memory-engine-readyz

status=$(curl -fsS --max-time 15 -o /tmp/memory-engine-home -w "%{http_code}" "$base/")
test "$status" = "200"

status=$(curl -fsS --max-time 15 -o /tmp/memory-engine-auth-boundary -w "%{http_code}" -X POST "$base/app/generate")
case "$status" in 4??) ;; *) echo "expected 4xx, got $status"; exit 1;; esac

curl -fsS --max-time 15 "$base/manifest.webmanifest" | jq -e \
  '.display == "standalone" and (.icons | length >= 2)' >/dev/null
curl -fsS --max-time 15 "$base/favicon.png" | file - | grep -q 'PNG image data, 192 x 192'
curl -fsS --max-time 15 "$base/apple-touch-icon.png" | file - | grep -q 'PNG image data, 180 x 180'
```

## Service sessions (machine consumers)

Agents, QA runs, and future service consumers authenticate without email.
`POST /v1/service-sessions` issues (or rotates) the account-scoped session
token for an allowlisted service account, gated by the operator admin token.
The surface is disabled entirely unless the app sets
`MEMORY_ENGINE_ADMIN_TOKEN`. Agents never read mail-provider archives; magic
links stay a human-only delivery channel.

Provisioning (operator, once):

1. Add the dedicated dogfood email to `MEMORY_ENGINE_AUTH_ALLOWED_EMAILS` in
   the app spec. Use a dedicated address — magic-link sign-in on the same
   email rotates the API session token and would revoke the service
   credential.
2. Set `MEMORY_ENGINE_ADMIN_TOKEN` as an app-spec secret. Custody: Mint
   (`secret://memory-engine/admin`); never commit or paste it.
3. Issue the credential and store it in Mint as
   `secret://memory-engine/dogfood`:

```sh
base="https://memory-engine-api-i2xcr.ondigitalocean.app"
curl -fsS --max-time 20 \
  -H 'content-type: application/json' \
  -H "x-admin-token: $MEMORY_ENGINE_ADMIN_TOKEN" \
  -d '{"email":"<dogfood-email>"}' \
  "$base/v1/service-sessions"
# -> {"accountId":"acct_...","sessionToken":"sess_..."}
```

The response maps onto the receipt variables below:
`MEMORY_ENGINE_ACCOUNT_ID=accountId`,
`MEMORY_ENGINE_SESSION_TOKEN=sessionToken`.

Rotation and revocation are the same call: every issue mints a fresh token
and the prior one fails with `403` immediately. To revoke without keeping a
usable credential, reissue and discard the response. Issuance is audited in
the app log (`service session issued account=...`).

## Waitlist (invite-beta first-run)

The signed-out landing page offers two actions: sign in (allowlisted emails,
existing flow) and join the waitlist (anyone, no account). `POST
/app/waitlist` records only a normalized email, a created/updated timestamp
pair, and a source tag (`"first-run"`) — no account, session, or generation
job is ever created. Joining is idempotent on normalized email and returns
the identical response whether the address is brand new, already on the
list, or already allowlisted, so the response can never be used to probe
registration or allowlist state. Rate limiting runs per normalized email and
per client IP, trusting only the edge-set `do-connecting-ip` header (falling
back to `x-real-ip`/`x-forwarded-for`) so a caller cannot forge its own quota
identity.

Storage is dual-backend, dispatched the same way as every other
`memory-engine-api` store: `MEMORY_ENGINE_POSTGRES_URL` set → Postgres
(`memory_engine_waitlist_entries` plus an append-only
`memory_engine_waitlist_audit_log` recording every join/invite/delete
transition); unset → the local file store only
(`crates/memory-engine-api-state/src/waitlist.rs`, `_waitlist.json` beside
the other store-root sidecars under `MEMORY_ENGINE_API_STORE_DIR`, with its
own `_waitlist_audit.jsonl` mirroring the same audit contract for local
dev/tests without a database). Production always runs Postgres-backed; the
join and admin routes below no longer return `503` there.

Operator surface, gated by `MEMORY_ENGINE_ADMIN_TOKEN` (the same admin token
used by service sessions) — list, export, mark invited, and delete, with no
direct SQL required:

```sh
base="https://memory-engine-api-i2xcr.ondigitalocean.app"

# List every entry as JSON.
curl -fsS --max-time 20 \
  -H "x-admin-token: $MEMORY_ENGINE_ADMIN_TOKEN" \
  "$base/internal/waitlist"
# -> [{"email":"...","createdAtMs":...,"updatedAtMs":...,"source":"first-run","invitedAtMs":null}, ...]

# Export the same rows as CSV (email,createdAtMs,updatedAtMs,source,invitedAtMs).
curl -fsS --max-time 20 \
  -H "x-admin-token: $MEMORY_ENGINE_ADMIN_TOKEN" \
  "$base/internal/waitlist/export"

# Mark one address invited. Idempotent: inviting an already-invited address
# again returns its existing invitedAtMs unchanged. 404 if the address never
# joined.
curl -fsS --max-time 20 \
  -H "x-admin-token: $MEMORY_ENGINE_ADMIN_TOKEN" \
  -H 'content-type: application/json' \
  -d '{"email":"<address>"}' \
  "$base/internal/waitlist/invite"

# Delete one address. Removes only the operational row; the audit log keeps
# a permanent record that the address joined (and was invited, if it was).
# 404 if the address is not present.
curl -fsS --max-time 20 \
  -H "x-admin-token: $MEMORY_ENGINE_ADMIN_TOKEN" \
  -H 'content-type: application/json' \
  -d '{"email":"<address>"}' \
  "$base/internal/waitlist/delete"
```

All four routes sit beside `/internal/scheduler/return-notifications`,
outside the versioned `/v1/*` contract — operator tooling, not a public API
surface. Marking invited only records that state on the waitlist row; it
does not grant access. Invited access still requires the operator to add the
address to `MEMORY_ENGINE_AUTH_ALLOWED_EMAILS` so the existing magic-link
sign-in flow accepts it.

## Queued generation (machine consumers)

Bearer-authenticated clients enqueue the same bounded, durable generation job
used by the browser app, then poll that account-scoped job until it reaches
`succeeded` or `failed`. No browser cookie, CSRF token, or synchronous model
request is involved. A duplicate POST while the same source is queued or
running returns `200` with the existing job id and `"coalesced": true`; a new
job returns `202`.
Admission-control rejections return `409`; a transient queue-store failure
returns `503` so machine clients can retry it.

```sh
set -euo pipefail

base="https://memory-engine-api-i2xcr.ondigitalocean.app"
account_id="${MEMORY_ENGINE_ACCOUNT_ID:?set MEMORY_ENGINE_ACCOUNT_ID}"
session_token="${MEMORY_ENGINE_SESSION_TOKEN:?set MEMORY_ENGINE_SESSION_TOKEN}"
source_id="${MEMORY_ENGINE_SOURCE_ID:?set MEMORY_ENGINE_SOURCE_ID}"

job_response="$(curl -fsS --max-time 20 \
  -H "authorization: Bearer $session_token" \
  -X POST \
  "$base/v1/accounts/$account_id/sources/$source_id/generation-jobs")"
job_id="$(printf '%s' "$job_response" | jq -er '.id')"

while :; do
  job_response="$(curl -fsS --max-time 20 \
    -H "authorization: Bearer $session_token" \
    "$base/v1/accounts/$account_id/generation-jobs/$job_id")"
  case "$(printf '%s' "$job_response" | jq -r '.status')" in
    succeeded) break ;;
    failed) printf '%s\n' "$job_response" >&2; exit 1 ;;
    queued|running|retry) sleep 1 ;;
    *) printf 'unexpected job response: %s\n' "$job_response" >&2; exit 1 ;;
  esac
done
printf 'scheduled cards: %s\n' "$(printf '%s' "$job_response" | jq -r '.cardCount')"
```

A succeeded job can report `cardCount: 0` when generation completed but no
draft passed the shared validation gate. That is a content-quality result, not
a queue failure; callers may inspect the source or submit revised material.

## Legacy v1 compatibility latency

Use this only when a ticket explicitly needs the synchronous compatibility
path. `/v1/accounts/{account_id}/sources/{source_id}/generate` remains a
legacy direct API surface and returns `409` on production Postgres hosts;
production consumers use the queued generation-job workflow above.
The command below uses an existing allowlisted v1 account/session, saves one article-sized source,
times the end-to-end direct request, records the response, and archives the
source. Production account creation is allowlist-protected, so do not use a
throwaway email here; export a pre-provisioned account id and session token
first.

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

Errors, health check-ins, and closed performance aggregates land in Canary
through one bounded worker queue. Every API 500 (`ApiFailure::internal`) is
reported as service `memory-engine-api`, environment `production`.
Performance observations are merged in process and export at most one batch
per minute for each of the trusted server, untrusted browser, and job
namespaces. Queue saturation, Canary failure, and shutdown deadlines remain
fail-open for request handling; delivery/drop/invalid counts ride in the
bounded aggregate schema. The same JSON is printed as
`authority=non_authoritative_debug` for immediate log inspection.

`POST /app/submit` is the only HTTP route with per-request performance
headers. Every response carries `X-Request-ID: req_<32 lowercase hex>` and a
content-free `Server-Timing` value with `request`, `total`, and `render`.
Browser-enhanced submits also carry the opaque `handoff` token. Postgres-backed
submits add `pgconnect`, `pgop`, and the cumulative `pgstmt` call count; those
metrics are omitted rather than reported as zero when the phase did not run.
The response is always `Cache-Control: no-store`. Health/readiness, static
assets, generation SSE, and the telemetry endpoint are intentionally outside
this instrumentation.

After a graded page is visible for two animation frames, the browser consumes
its short-lived same-tab handoff and posts one strict, content-free receipt to
`POST /app/performance/submit`. Canary receives the trusted server route total
separately from the untrusted browser tap-to-ack and graded-visible durations;
the browser series carries only a coarse `mobile`, `tablet`, or `desktop`
viewport. Missing APIs, stale or mismatched handoffs, BFCache restores, and
malformed timings fail closed without delaying or changing review submission.

The production image includes a bounded live receipt. Open a DigitalOcean
component console and run it without printing either encrypted value:

```sh
doctl apps console 5ab05b73-9265-43c9-a01c-fef53f5f46a4 api
memory-engine-canary-receipt emit-openapi
```

The receipt times a real loopback `GET /v1/openapi.json`, queues one closed
server observation, drains the worker, and exits nonzero on HTTP or delivery
failure. Readback uses a distinct service-bound read credential; the
production app keeps ingest-only authority:

```sh
CANARY_READ_ENDPOINT=https://canary.mistystep.io \
CANARY_READ_API_KEY=... \
CANARY_READ_SERVICE=memory-engine-api \
cargo run -p memory-engine-canary --bin memory-engine-canary-receipt -- readback
```

Never reuse or promote the app's ingest key for readback. The deterministic
admission-overhead receipt is
`cargo run -p memory-engine-canary --bin memory-engine-canary-receipt -- overhead`;
it fails above a 5 ms p95.

## Secrets

The DigitalOcean app owns the only live values below. Keep every secret typed as
an encrypted App Platform variable and never copy its value into this repo.

| Secret | Purpose |
| --- | --- |
| `MEMORY_ENGINE_POSTGRES_URL` | Neon pooled connection string (project `twilight-brook-49749008`, `memory-engine-prod`). |
| `MEMORY_ENGINE_AUTH_ALLOWED_EMAILS` | Comma-separated allowlist; account creation and magic links refuse other emails. |
| `MEMORY_ENGINE_RETURN_UNSUBSCRIBE_SECRET` | Stable secret for HMAC-signed, seven-day, account/email-scoped reminder unsubscribe links. |
| `MEMORY_ENGINE_RETURN_NOTIFICATION_SCHEDULER_ENABLED` | Optional kill switch; set `false` to disable scheduled reminder sweeps. |
| `MEMORY_ENGINE_RETURN_NOTIFICATION_MANUAL_TOKEN` | Operator-only token for bounded manual scheduler runs; store encrypted. |
| `MEMORY_ENGINE_RETURN_NOTIFICATION_BATCH_SIZE` | Optional bounded sweep size, default 100 and capped at 1000. |
| `MEMORY_ENGINE_RETURN_NOTIFICATION_SCHEDULER_INTERVAL_SECONDS` | Optional sweep interval, default 900 seconds and capped at 86400. |
| `MEMORY_ENGINE_AUTH_LINK_OUTBOX_PATH` | Magic-link outbox file (no email provider wired yet; see Login). |
| `OPENROUTER_API_KEY` | Enables model-backed generation for pasted prose; absent → structured-block parsing only. |
| `MEMORY_ENGINE_GENERATION_MODEL` | Optional model override (default `google/gemini-3.5-flash`; see docs/evals/). |
| `CANARY_ENDPOINT` / `CANARY_API_KEY` | Ingest-only Canary error, check-in, and bounded performance export; absent → reporting is a no-op. |
| `MEMORY_ENGINE_ENVIRONMENT` | Environment label on Canary error events. |

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
into the image at `/usr/local/bin/send-magic-link`), which sends through
Resend from `onboarding@resend.dev` — deliverable only to the Resend account
owner's address, which matches the solo-dogfood allowlist. Activate it by
setting `RESEND_API_KEY` and
`MEMORY_ENGINE_AUTH_MAILER_COMMAND=/usr/local/bin/send-magic-link` as encrypted
variables in the DigitalOcean app spec, updating the app, and rerunning the
deployed smoke. Do not put the key in a shell command or checked-in spec.

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

A failed send surfaces as a 500 and therefore lands in Canary. The sender
address is the `MEMORY_ENGINE_MAIL_FROM` secret (default
`Memory Engine <onboarding@resend.dev>`); switching it is a secret change, not
a code edit — see Deliverability below.

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
OS descriptor lock. Every writer (save/disable preference, claim, complete,
release) acquires the lock with a blocking `flock`, so a contending writer
waits for the lock instead of skipping the account outright; after acquiring
it, the writer re-reads the account's current on-disk state and re-checks
eligibility (enabled, claim ownership, retry timing) before mutating, so a
recheck always sees the latest committed state rather than a stale in-memory
view. The lock path is never deleted as part of ownership release, so stale
paths are harmless, and a process crash releases the descriptor for the next
waiter. The same libc-backed helper protects the repository-owned file
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

**Production receipt gate (open):** the manual trigger above has been proven
locally and against real Postgres, and `/healthz` confirms the deployed
scheduler is live (`returnNotificationScheduler.enabled: true`), but no
production-safe receipt has yet been executed proving the *deployed*
scheduler — not a local run or a page render — initiated an allowlisted
reminder end to end (provider send + delivery evidence). Card memory-engine-097
criterion 6 stays open until an operator runs the manual trigger above against
production with `MEMORY_ENGINE_RETURN_NOTIFICATION_MANUAL_TOKEN`, against one
allowlisted account with a genuinely due card, and links the resulting send
receipt (provider log line and/or file-outbox `due-count` entry) to the card.
Do not mark memory-engine-097 complete without that receipt.

**Production probe receipt (2026-07-21, criterion 6 remains open):** A DigitalOcean App Platform console session on app `memory-engine-api` component `api` observed the in-process manual-token variable was absent (reported only as `token_absent`; no token bytes were printed or stored), so the guarded manual command refused before making an authenticated request. Independent production POST probes with an absent token and a known-invalid token both returned `403` with the same authorization error. The deployed `/healthz` returned `200` with `returnNotificationScheduler.enabled=true`, `running=false`, `lastRunAtMs=1784688531490`, `lastSuccessAtMs=1784688531490`, and `failureCount=0`. DigitalOcean run logs reported `return notification scheduler examined=0 due=0 sent=0 skipped=0 failed=0 truncated=false` for the observed sweeps. No allowlisted account was examined and no reminder was initiated, so this is a truthful liveness/zero-eligibility receipt, not proof of criterion 6; do not close memory-engine-097 until an operator configures the existing encrypted token and a genuinely due allowlisted account produces the required provider/outbox send evidence.

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
5. **Switch the sender** by updating the encrypted
   `MEMORY_ENGINE_MAIL_FROM=Memory Engine <noreply@mail.<domain>>` App Platform
   variable; no script edit is required. Deploy the updated app spec and rerun
   the smoke.
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
per client IP (`do-connecting-ip`, then `x-real-ip`, then the first
`x-forwarded-for` value). Rejected requests return `429` with the generic
message "Too many sign-in attempts. Try again later."; they do not write an
outbox row or reveal whether an email is allowlisted.

The browser session is server-side. `POST /app/logout` requires the same CSRF
token as other app mutations, revokes the stored browser session, and clears
the `__Host-memory_engine_session` cookie with `Max-Age=0`. Reusing the old
cookie after logout must return `401`, including after a process restart.
