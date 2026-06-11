# Wire real wall-clock time through the production boundary

Priority: P0 · Status: pending · Estimate: M

## Goal

Production behaves correctly across real days: reviews space out per FSRS,
magic links and sessions actually expire, and the kernel stays pure via an
injected clock.

## Oracle

- [ ] `api_study_now()` (or its replacement) returns wall-clock ms in the
      served binary; `DEFAULT_BETA_STUDY_NOW` survives only in tests/fixtures.
- [ ] Regression test: a correct answer schedules the unit due in the future;
      advancing the injected clock past the due date surfaces it in
      `/app/next`, and not before.
- [ ] Regression test: an auth challenge older than `AUTH_CHALLENGE_TTL_MS`
      is rejected; an expired browser session is rejected server-side.
- [ ] Live QA receipt on the deployed app: submit a review, confirm it is not
      immediately due again.

## Notes

Root cause: time was injected for kernel purity but never bound to real time
at the boundary — `crates/memory-engine-api/src/lib.rs:2358` returns the
frozen constant from `memory-engine-study/src/lib.rs:27`. Every call site
(auth TTL line 276, session checks line 703, scheduling, idempotency) shares
the frozen clock, so nothing expires and nothing spaces. This silently
invalidates the FSRS value proposition and weakens auth. Fix at the boundary;
do not put `SystemTime` in core.

## Children

1. Bind the api binary's clock to wall time; keep frozen clock available to
   tests via injection.
2. Expiry regression tests (auth challenge, browser session, magic-link
   single-use under real time).
3. Multi-day scheduling test through the study session (clock advanced).
4. Deployed-app QA receipt under docs/qa/.
