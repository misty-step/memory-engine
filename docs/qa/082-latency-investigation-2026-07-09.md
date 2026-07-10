# 082 — production latency investigation

Ticket: memory-engine-082. Operator's first live dogfood (2026-07-09) reported
every review interaction on the DigitalOcean primary
(`https://memory-engine-api-i2xcr.ondigitalocean.app`) as laggy — page loads
and MCQ submits both felt slow. This is the receipt: production timing
breakdowns, a local live measurement that isolates app-side cost from
network/TLS cost, the root cause, and shaped follow-up recommendations. No
production accounts were created and no reviews were posted against
production; the account-creation and review-submit boundary was only probed
read-only (`GET /healthz`, `GET /`, static assets) per the ticket's guardrail.

## 1. Production timing (curl, 5x each, `-w` timing breakdown)

`namelookup` / `connect` / `appconnect` are cumulative from request start
(so `appconnect - connect` is the TLS handshake, `starttransfer - appconnect`
is server + first-byte time). All requests were warm (no cold start observed;
DO App Platform keeps the single instance running continuously).

```
=== GET /healthz x5 ===
[1] namelookup=0.044 connect=0.069 appconnect=0.096 starttransfer=0.225 total=0.225
[2] namelookup=0.002 connect=0.018 appconnect=0.057 starttransfer=0.170 total=0.170
[3] namelookup=0.003 connect=0.014 appconnect=0.056 starttransfer=0.164 total=0.167
[4] namelookup=0.002 connect=0.025 appconnect=0.057 starttransfer=0.172 total=0.172
[5] namelookup=0.002 connect=0.021 appconnect=0.068 starttransfer=0.211 total=0.211

=== GET / x5 ===
[1] appconnect=0.047 starttransfer=0.211 total=0.211
[2] appconnect=0.047 starttransfer=0.147 total=0.147
[3] appconnect=0.050 starttransfer=0.208 total=0.209
[4] appconnect=0.082 starttransfer=0.244 total=0.244
[5] appconnect=0.049 starttransfer=0.203 total=0.204

=== GET /static/ledger.css x5 ===
[1] appconnect=0.052 starttransfer=0.093 total=0.093
[2] appconnect=0.056 starttransfer=0.116 total=0.116
[3] appconnect=0.042 starttransfer=0.094 total=0.095
[4] appconnect=0.060 starttransfer=0.176 total=0.188
[5] appconnect=0.043 starttransfer=0.123 total=0.123

=== GET /static/app.js x5 ===
[1] appconnect=0.056 starttransfer=0.107 total=0.110
[2] appconnect=0.058 starttransfer=0.099 total=0.099
[3] appconnect=0.051 starttransfer=0.128 total=0.128
[4] appconnect=0.049 starttransfer=0.171 total=0.175
[5] appconnect=0.054 starttransfer=0.298 total=0.300
```

Reading it: TLS handshake (client here → DO `nyc` region) is consistently
~45–95ms — real geography/TLS cost, not fixable from the app. `/healthz`
touches no storage at all (static JSON) yet still costs ~100–150ms of
server time beyond TLS-complete, and `/` (a full signed-out render, no
Postgres call for an anonymous visitor) shows the same band. That residual,
non-DB server-side cost on trivial routes is consistent with the app
instance being resource-constrained (see §3) rather than a query problem —
but it is a minor contributor next to §2.

## 2. Local live measurement: the dominant cost is per-request Postgres query volume, not network

Read-only production probing can't show what an authenticated review action
costs, and the ticket asks not to create accounts or post reviews against
production. Instead this reproduces the *exact* production code path locally:
`memory-engine-api`'s release binary against a real Postgres (`postgres:17-
alpine` in Docker, `sslmode=disable`, same machine — i.e. near-zero network
RTT), with `log_statement=all` so every SQL statement the app issues is
directly counted, not inferred.

Setup: created an account via the real magic-link flow (`POST /app/account`
→ debug link → `GET /app/login/verify`), captured a two-card NATO source,
generated (deterministic structured-block provider, no model network call),
then drove `/app/next` and `/app/submit` and diffed the Postgres log around
each request.

```
/app/next   (warm, 3 reps): time_total = 0.116s, 0.122s, 0.128s
                              48 executed statements per request
/app/submit (warm, 1 rep):  time_total = 0.885s
                              70 executed statements in that single request
```

Even over loopback with a warm connection, one MCQ submit fires **70
sequential SQL round trips** and takes ~885ms. Query-shape breakdown for the
submit request (`execute` count):

```
13  SELECT state FROM memory_engine_schedules
 9  SELECT record FROM memory_engine_review_units
 5  SELECT span FROM memory_engine_reference_spans
 5  SELECT run FROM memory_engine_generation_runs
 5  SELECT review_unit_id, state FROM memory_engine_schedules
 5  SELECT receipt_key, ... FROM memory_engine_applied_reviews (idempotency)
 5  SELECT note FROM memory_engine_concept_reference_notes
 5  SELECT draft FROM memory_engine_generated_prompt_drafts
 5  SELECT document FROM memory_engine_source_documents
 5  SELECT attempt FROM memory_engine_attempts
 2  SELECT 1 FROM memory_engine_applied_reviews / memory_engine_review_units
 2  INSERT INTO memory_engine_accounts ... ON CONFLICT DO NOTHING
 1  INSERT INTO memory_engine_schedules / memory_engine_attempts / memory_engine_applied_reviews (the actual writes)
```

**Root cause: `BetaStudySession` re-fetches the entire account snapshot (9
separate `SELECT`s — sources, reference spans, drafts, review units,
schedules, attempts, generation runs, applied reviews, concept notes) on
almost every method call, not once per session.** In
`crates/memory-engine-study/src/lib.rs`:

- `BetaStudySession::from_store` (line ~494) calls `store.snapshot()` once at
  construction just to classify `Empty` vs `Drafting`.
- `submit_answer_with_idempotency_key` / `start()` (`select_next` at line
  ~1224, `view()` at line ~1095) each call `self.store.snapshot()` again, plus
  separate `list_queue_candidates()` reads.

For the file-backed store (`crates/memory-engine-persistence`) this is free —
`BetaPersistenceStore::snapshot()` (line 467) is `self.data.clone()`, an
in-memory clone of data already loaded once at `open()`. The abstraction was
designed assuming `snapshot()` is cheap, which is true for the file store and
was never adjusted for the Postgres-backed host
(`crates/memory-engine-persistence-postgres`), where `AccountStudyStore::
snapshot()` (line 563) issues 9 live network round trips *every single call*,
with no caching and no connection pool. That assumption-mismatch, not
network distance, is the dominant cost: 5 full-account re-snapshots plus
extra queue reads (≈70 round trips) for one MCQ answer, even with
near-zero-latency Postgres. Against the real production path — Neon in
`aws-us-east-2`, the DO app in `nyc` — each of those round trips pays real
cross-region WAN latency instead of loopback, which is consistent with "every
review interaction ... felt laggy."

A second, smaller redundancy in the same family: `crates/memory-engine-api-
state/src/lib.rs`'s `with_postgres_account` (line ~1500) calls `account.
ensure_account(now_ms)` — an `INSERT ... ON CONFLICT DO NOTHING` — on *every*
authenticated Postgres request, even though `AccountRegistry::require_account`
(`registry.rs` line ~757) already confirms the account exists via an
in-process cache before ever reaching storage. It's idempotent and cheap
per-call, but it's one more guaranteed round trip on every hot-path request.

A third, render-layer redundancy was also found but is out of this PR's
scope (see §4): `crates/memory-engine-api-render/src/render.rs`'s
`render_account_page` (line ~39) unconditionally calls `state.
list_app_sources(account)` (another full-account-snapshot Postgres fetch) on
every page render — including every `/app/next`, `/app/submit`, `/app/reveal`,
`/app/skip`, `/app/snooze`, `/app/bridge`, `/app/delete` response, even though
`render_signed_in` (line ~178) only reads `sources` on the workspace branch,
never on the active-review-card branch that those routes render into.

## 3. DO app spec: instance sizing and region

```
doctl apps spec get 5ab05b73-9265-43c9-a01c-fef53f5f46a4
```

- `region: nyc`, `instance_count: 1`, `instance_size_slug: apps-s-1vcpu-
  0.5gb` — the smallest paid App Platform tier, one instance, no autoscaling.
- Postgres (Neon, via `neonctl projects list`): project `twilight-brook-
  49749008`, region `aws-us-east-2` (Ohio). `nyc` app ↔ `us-east-2` DB is a
  same-coast, modest-latency hop (tens of ms), not the multi-hundred-ms
  cross-continent case — consistent with §1's TLS/connect numbers being
  network-plausible and not the dominant driver.
- No connection pooler (pgbouncer/deadpool/bb8) anywhere in the stack —
  `memory-engine-persistence-postgres` opens a fresh TCP+TLS connection *per
  storage call* (`connect_client`, `lib.rs` line ~250, doc-commented as
  intentional with retry). Combined with §2's finding, this means production
  pays connection setup cost on top of the ~70 sequential round trips for a
  single submit, not instead of it.
- The single small instance explains §1's residual ~100–150ms on
  `/healthz`/`/` (no DB call) reasonably well, but is a minor contributor
  next to §2's query-volume finding, which dominates every authenticated
  study action.

**Verdict: app time, driven by per-request Postgres query volume
(`BetaStudySession`'s repeated full-snapshot fetches over an unpooled,
per-call connection), not network/TLS/region.** Region and instance size are
real but secondary; even a same-machine Postgres in §2 reproduced the
"laggy" feel.

## 4. Fix landed in this PR vs. shaped follow-up

No latency fix is landed in this PR. The smallest, safest candidate (drop
the redundant `list_app_sources` fetch in `render_account_page` when
rendering an active review card) lives in `crates/memory-engine-api-
render/src/render.rs`, which this ticket explicitly scopes to another lane
("Do NOT touch ... render.rs"). The dominant fix — collapsing
`BetaStudySession`'s repeated `snapshot()` calls into one fetch per session
— touches a shared, heavily-used abstraction (`memory-engine-study`, used by
both the file store and the Postgres store) across ~10 call sites and
changes a correctness-sensitive invariant (whether a later call in the same
session sees writes made by an earlier call in that session); landing it
without a dedicated red/green cycle proving cache-invalidation correctness
would be exactly the kind of rushed fix Part 1 of this ticket exists to
prevent. Recommended follow-up tickets, in priority order:

1. **Cache one account snapshot per `BetaStudySession` and invalidate it
   only on writes the session itself makes**, instead of re-fetching on
   every read. This is the ~70→~10-round-trip win; needs its own TDD pass
   with tests that specifically assert staleness cannot leak across a
   session's own writes (the file store's "snapshot is free" assumption
   made this invisible for years — a regression test against the Postgres
   adapter, not just the file adapter, should gate it).
2. **Drop the render-layer `list_app_sources` fetch when a review card is
   active** (owned by the render.rs lane) — small, safe, and independently
   shippable; five hot routes (`next`, `submit`, `reveal`, `skip`, `snooze`,
   `bridge`) skip one full snapshot fetch each.
3. **Skip the per-request `ensure_account` write** once the in-process
   account cache (`AccountRegistry`'s `data.accounts`) has confirmed the
   account exists, instead of re-issuing an `INSERT ... ON CONFLICT DO
   NOTHING` on every authenticated request. Smaller win, needs the cache
   threaded into `with_postgres_account` (currently a free function keyed
   only on `database_url`/`account_id`).
4. **Introduce a Postgres connection pool** (e.g. `deadpool-postgres` or
   `bb8`) so hot requests reuse a warm connection instead of paying
   connect+TLS setup per storage call — currently masked by #1's bigger
   cost, but becomes the next bottleneck once #1 lands.

## 5. Did the double-generation bug actually duplicate cards in production, and what cleanup does the operator need?

**Yes — this is provable from the generation code path, without touching
production data.** `BetaStudySession::generate`/`generate_with_provider`
(`memory-engine-study/src/lib.rs` lines ~679–709) is **not idempotent per
source**: every call stamps a brand-new `run_id` (`format!("study-run-{}",
snapshot.generation_runs.len() + 1)`) and unconditionally runs generation
again for the requested source ids, appending a fresh `generation_run` and a
fresh set of `generated_prompt_drafts`/review units on top of whatever
already exists. Before this PR's fix, two enqueued jobs for the same
account+source (the "Create review" double-press dogfood repro) each called
`run_generation_job` → `storage.generate_source` → `generate_with_provider`
independently — both jobs generated, both auto-approved, both scheduled
their own full set of cards for the same source content. For the "47 US
presidents" source, that means the production store almost certainly holds
**two `generation_run` rows and two full sets of review units/schedules**
for that one source (doubling the deck for it specifically — no other
source or account is affected, since generation is scoped per source+
account).

**Operator cleanup, using only existing, already-shipped affordances (no
new code, no direct production data edits needed):**

1. Open the app, find the "47 US presidents" source in Saved material.
2. Press **Remove** (`POST /app/source/archive`). `archive_source`
   (`memory-engine-study/src/lib.rs` line ~559) archives the source *and*
   retires every review unit whose drafts reference it — both generation
   runs' worth, not just the latest — so this one action clears every
   duplicate card for that source in a single step.
3. Re-paste the same material as a new source and press **Create review**
   once. With this PR's coalescing fix live, a second press (or an
   accidental double-tap) now returns "Already generating this source."
   instead of enqueueing a duplicate job, so the doubling cannot recur for
   this or any other source.

No other source or account is implicated — the duplication is scoped
exactly to sources that had two "Create review" presses land while a job was
still queued/running, which per the operator's report is just the one
source from the 2026-07-09 dogfood session.
