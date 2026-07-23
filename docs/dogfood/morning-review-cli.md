# Morning Review CLI Dogfood

Refs-Powder: memory-engine-070

## Purpose

`crates/memory-engine-review` is the second dogfood client, and the first one
that talks to the deployed `memory-engine-api` over the network instead of an
in-process fixture (`memory-engine-cli`) or a scripted contract run
(`memory-engine-contract`). It is deliberately thin: authenticate once, then
loop `review/next` -> answer -> `review/submit` until the account's due queue
is empty, logging enough locally to check later whether this became a real
daily habit.

It adds **no new server surface**. Everything it computes (due-count-zero
completion, cold-recall accuracy) is derived from fields the v1 contract
already returns (`dueCount`, `grade`, `scheduleChange.before/after`).

## Auth model

The v1 contract is Bearer-token, not cookie/browser-session. Anonymous account
creation is not a client contract: credentials must be provisioned through the
invite magic-link flow or the operator-gated service-session flow. Every review
call sends `Authorization: Bearer <sessionToken>`.

Import a pre-provisioned credential pair once:

```sh
MEMORY_ENGINE_ACCOUNT_ID=acct_... \
MEMORY_ENGINE_SESSION_TOKEN="$SESSION_TOKEN" \
cargo run -p memory-engine-review -- login \
  --base-url https://memory-engine-api-i2xcr.ondigitalocean.app \
  --account-id $MEMORY_ENGINE_ACCOUNT_ID \
  --session-token $MEMORY_ENGINE_SESSION_TOKEN
```

The command writes `~/.memory-engine/review/credentials.json` (mode `0600`,
never committed). Environment variables with the same names override the file,
so production credentials can stay in the shell or secret manager. The CLI never
queries persistence for a token and never creates an account.

## Commands

```sh
cargo run -p memory-engine-review -- login \
  --account-id acct_... --session-token "$SESSION_TOKEN"
cargo run -p memory-engine-review               # runs the review loop (default subcommand)
cargo run -p memory-engine-review -- streak      # 30-day streak + cold-recall report
cargo test -p memory-engine-review
```

`bun run rust:morning-review` / `bun run rust:morning-review:streak` are
registered in `package.json` for the same two commands.

## Falsifier

**Claim:** a human can run one command each morning, answer typed prompts,
and reach `dueCount == 0` in under two minutes, with local evidence that
later proves whether this became a real habit.

**Would falsify it:**

- The loop cannot reach `dueCount == 0` without reaching into API internals
  or duplicating service logic (same falsifier `memory-engine-contract`
  already cleared for the scripted case; this proves it under real stdin/
  human-shaped interaction too).
- The 30-day streak or cold-recall numbers cannot be reconstructed from
  fields the v1 contract already returns — would mean new server surface is
  actually required, contradicting this ticket's non-goal.

**Where the data lives:** `~/.memory-engine/review/streak.ndjson` (one JSON
line per graded attempt, one per completed session — append-only, never
truncated by this CLI).

**The command that reports it:** `cargo run -p memory-engine-review --
streak [--days N]`. It reports, over the trailing `N`-day window (default
30): hit rate (days with a completed session / N), the current consecutive-
day streak ending today-or-yesterday, and cold-recall accuracy (attempts on
review units where `scheduleChange.before.reps >= 1` — i.e., not the very
first exposure — correct / total). A day only counts as "completed" if the
loop actually reached `dueCount == 0`; hitting the `--max-cards` safety cap
or an aborted stdin does not log a session line, by design.

## Self-run transcript (historical, 2026-07-04)

The transcript below preserves the endpoint used during that dated run. It is
evidence, not a current invocation; the CLI default and `docs/runbook.md` point
to the DigitalOcean runtime.

Two runs were exercised for this ticket:

### 1. Production auth boundary (real, deployed API, read-only)

Anonymous account creation is not a client contract. The v1 account-session route is
operator-gated; browser sign-in uses the invite magic-link flow. This receipt
therefore does not exercise anonymous account creation or email delivery.

This ticket deliberately did **not** pull the operator's real production
session token to run the full review loop against his live account: doing so
would submit answers that mutate his actual FSRS schedule state (reps,
lapses, due dates) on real content, which isn't reversible. That's an
operator decision, not an engineering gap — see Residual Risk.

### 2. Full functional loop (real binary, local file-store API, not a mock)

A local `memory-engine-api` was started exactly per `docs/runbook.md`'s local
file-store pattern (not a Bun/JS stub — the same Rust binary that runs in
production, pointed at a scratch store dir):

```sh
$ MEMORY_ENGINE_ENABLE_FILE_STORE=true \
  MEMORY_ENGINE_API_STORE_DIR=/tmp/me-review-dogfood/store \
  MEMORY_ENGINE_AUTH_ALLOWED_EMAILS=dogfood-morning-review@example.com \
  MEMORY_ENGINE_AUTH_LINK_OUTBOX_PATH=/tmp/me-review-dogfood/outbox.tsv \
  MEMORY_ENGINE_RETURN_UNSUBSCRIBE_SECRET=local-dogfood-secret \
  HOST=127.0.0.1 PORT=18099 ./target/debug/memory-engine-api &
Memory Engine API listening on http://127.0.0.1:18099
```

An account, one source, and one approved review unit were seeded through the
same v1 endpoints the CLI itself uses (`POST /v1/accounts`, `.../sources`,
`.../sources/{id}/generate`, `.../drafts/{id}/keep`) — the seeding is
outside this ticket's scope (source ingestion is a different dogfood
surface); this only proves the review-loop half.

Then the actual `memory-engine-review` binary — not `cargo test`, a real
terminal invocation:

```sh
$ ./target/debug/memory-engine-review login --base-url http://127.0.0.1:18099 \
    --account-id acct_fc634ded5803c446 --session-token sess_***redacted***
Saved credentials for account acct_fc634ded5803c446 to /tmp/me-review-dogfood/home/review/credentials.json (base url http://127.0.0.1:18099).

$ printf 'ALFA\n' | ./target/debug/memory-engine-review review
[1 due] What is the NATO phonetic alphabet word for A?
  1. ALFA
  2. BRAVO
  3. CHARLIE
>   -> correct (rating 3)

All caught up. Reviewed 1 card(s).

$ printf '' | ./target/debug/memory-engine-review review
All caught up. Reviewed 0 card(s).

$ ./target/debug/memory-engine-review streak
{
  "windowDays": 30,
  "completedDays": 1,
  "hitRate": "3%",
  "currentStreakDays": 1,
  "currentStreakAsOf": "2026-07-04",
  "coldAttempts": 0,
  "coldCorrect": 0,
  "coldRecallRate": "n/a"
}
```

Raw log this produced (`~/.memory-engine/review/streak.ndjson`, one line per
event):

```json
{"type":"attempt","tsMs":1783153272729,"accountId":"acct_fc634ded5803c446","reviewUnitId":"generated-quiz-src-1d709b00be893b06-1-nato-letter-a","verdict":"correct","rating":3,"isCorrect":true,"cold":false,"dueCountAfter":0}
{"type":"session","tsMs":1783153272730,"accountId":"acct_fc634ded5803c446","reviewedCount":1,"dueCountAtStart":1}
```

`"cold":false` is correct here — this was the review unit's first-ever
exposure (`scheduleChange.before` was `null`), so it does not count toward
cold-recall, which is exactly the intended distinction from first-exposure
recognition. The wrong-answer feedback path (`-> wrong (rating 1) —
expected: <answer>`) was smoke-tested separately by resubmitting an
incorrect answer against an already-graded unit and confirming
`expectedAnswer` renders in the CLI output; a from-scratch cold-recall
transcript (a unit reviewed for a second time, `scheduleChange.before.reps
>= 1`) needs a longer-running local session and is left as the natural
follow-on once real usage accumulates repeat reviews.

Credential provisioning is intentionally outside this client. The local end-to-end test `review_loop_drives_a_real_local_api_end_to_end` explicitly seeds a fixture account before starting the real axum router, then exercises the review loop over HTTP. This keeps the client honest without reintroducing anonymous account creation.

## Stayed Outside This Client

- source creation/generation/draft-approval (a different dogfood surface;
  this client assumes a populated due queue)
- voice input (typed only; a later rung)
- any new server-side streak/cold-recall field or endpoint
- production account self-service; credentials are provisioned outside this client

## Residual Risk

- The full review loop was proved against a real local instance of the
  production binary, not against the deployed Fly URL with the operator's
  real account, because doing so would mutate his live FSRS schedule state
  on real content from an unattended agent run. Running it once against the
  actual deployment with his own answers (a "real morning") is the natural
  next verification step and is an operator action, not an engineering gap.
- `--max-cards` (default 200) is an untested-in-practice safety cap; no due
  queue in this ticket's testing ever approached it.
- The streak/cold-recall definitions are this ticket's first cut. "Cold" =
  `scheduleChange.before.reps >= 1` (has been graded at least once before);
  a stricter definition (e.g. requiring `before.state == 2`, i.e. already
  graduated out of initial learning) is a reasonable future refinement if
  this first cut proves too permissive once there's real longitudinal data.
- A fresh-context review of this diff (see PR) caught two real bugs before
  merge, both fixed with regression tests: `streak_report` was truncating
  the *current streak* to the `--days` reporting window (a real 45-day
  streak reported as 30 with `--days 30`), and `write_credentials` briefly
  created the credentials file under the ambient umask before narrowing it
  to `0600`, instead of creating it at `0600` atomically. Neither survives
  in the shipped code — `current_streak_is_not_truncated_by_the_reporting_window`
  and the rewritten `open_restricted` cover them — but they're recorded here
  as the class of bug this thin a client can hide without a second pair of
  eyes on the diff.
