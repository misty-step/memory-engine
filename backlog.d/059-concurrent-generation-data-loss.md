# Concurrent generation jobs clobber each other (data loss)

Priority: P1 · Status: done · Estimate: M

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

- [x] A regression test fires several concurrent captures for one account through
      the real spawned worker and asserts every scheduled card persists — no lost
      update. `concurrent_generations_for_one_account_do_not_clobber_each_other`
      reproduces the clobber on the unfixed code (8 reported / 5 on disk) and is
      deterministically green with the fix.
- [x] Each job's reported `card_count` equals the review units actually persisted
      — the test asserts `sum(card_count) == reviewUnits on disk`, so the activity
      log can't diverge from durable state.
- [x] ~~Approve only the job's own source's drafts~~ — **subsumed by
      serialization, not implemented separately.** Per-account serialization means
      each capture runs to completion (its drafts already approved) before the
      next starts, so the account-wide approve only ever sees the current source's
      pending drafts and `card_count` is already correct. A true source-scoped
      filter would need a source id on `BetaStudyDraftRow` (cross-crate) and is
      not the data-loss cause; deferred.
- [x] Mechanism: a per-account keyed `Mutex` (`AccountRegistry::store_lock`) held
      across the whole `run_generation_job`. Different accounts never contend.
- [x] Production exposure: prod is **Postgres** (`MEMORY_ENGINE_POSTGRES_URL`
      deployed), whose transactions already covered concurrency — the clobber was
      specific to the **file-backed host** (local dogfood). The lock is
      backend-agnostic, so it fixes the file host and is harmless belt-and-
      suspenders on Postgres.

## Notes

The 26 lost NATO cards are unrecoverable (never written); re-capture works and,
post-fix, won't clobber. Strictly the durability/concurrency fix — not the
broader generation-quality work (060/061).

**Residual (follow-up, out of scope):** the lock guards `run_generation_job`
only. A user review or approve (HTTP handler) that mutates the store while a
background generation runs is still an unsynchronized writer on the file host —
a narrower window (one user, rare overlap) than the worker's 4-way concurrency.
The same `store_lock` extends to those mutators if it proves to bite; filed as a
note rather than widening this P1.

## Children

1. Failing regression test: concurrent captures, one account, assert no lost update.
2. Per-account write serialization (or atomic RMW) around generation persistence.
3. Scope `run_generation_job` approval to its own source.
4. `card_count` ↔ persisted-review-unit invariant.
5. Confirm + document production host mode and exposure.
