# Concurrent generation jobs clobber each other (data loss)

Priority: P1 · Status: pending · Estimate: M

## Goal

Two near-simultaneous captures for the same account silently lose one source's
cards. Dogfood 2026-06-23: a NATO capture generated and approved **26 cards**
(the job reported `card_count=26`), but a second capture (Apostles Creed) that
overlapped it clobbered the shared `study.json`, leaving **0** NATO review
units persisted. The learner sees "26 cards · scheduled" in the activity log
and then only 6 cards exist. Make concurrent generation durable: no capture may
destroy another's persisted cards.

## Root cause (diagnosed)

- Persistence rewrites the **entire** `study.json` per save
  (`serde_json::to_string_pretty` → `fs::write`,
  `crates/memory-engine-persistence/src/lib.rs:778`), and `open_study_session`
  takes **no lock**. The only `Mutex` in the API guards account *metadata*, not
  the study store.
- 055 made generation concurrent (worker semaphore = `MAX_CONCURRENT_JOBS=4`).
  Two jobs each read the file, mutate their own copy, and write it back —
  classic lost-update race. The artifacts confirm it: `study-run-1` (NATO) has
  `draftIds: []`, `completedAt: null`; `study-run-2` (Apostles) is complete and
  owns all 7 drafts + 6 review units; yet the NATO job returned 26.
- Secondary bug: `run_generation_job` (`registry.rs:258`) reads an
  **account-wide** `study_view` and approves *every* pending draft, not just its
  own source's — racy and wrong under concurrency, and it lets the activity
  `card_count` diverge from durable reality.

This is a sibling of 057 (concurrent writers + whole-file write); the fix is the
same shape applied to the study store rather than the job queue.

## Oracle

- [ ] A regression test fires ≥2 concurrent captures for one account through the
      real spawned worker and asserts **every** source's cards fully persist —
      no lost update. The test reproduces the clobber on the current code first
      (red), then passes.
- [ ] Each job's reported `card_count` equals the number of review units
      actually persisted for that source — the activity log cannot diverge from
      durable state.
- [ ] `run_generation_job` approves only its **own** source's drafts, not the
      account-wide pending set.
- [ ] Mechanism documented: per-account serialization of the store mutation
      (e.g. a keyed async lock held across generate→persist) or an equivalent
      atomic read-modify-write. In-flight concurrency across *different* accounts
      is preserved.
- [ ] Production exposure confirmed: state whether the prod host is Postgres
      (DB transactions cover it) or file-backed (same fix applies). The local
      dogfood host is file-backed and is affected today.

## Notes

The 26 lost NATO cards are unrecoverable (never written); re-capture works and,
post-fix, will not clobber. Do not widen scope to the broader generation-quality
work (060/061) — this ticket is strictly the durability/concurrency fix.

## Children

1. Failing regression test: concurrent captures, one account, assert no lost update.
2. Per-account write serialization (or atomic RMW) around generation persistence.
3. Scope `run_generation_job` approval to its own source.
4. `card_count` ↔ persisted-review-unit invariant.
5. Confirm + document production host mode and exposure.
