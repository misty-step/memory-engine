# Reconcile repo docs and agent harness with the post-cutover reality

Priority: P2 · Status: pending · Estimate: S

## Goal

A cold agent (or human) reading the repo's ground-truth docs gets the
current Rust/Fly reality, not TypeScript-era instructions.

## Oracle

- [ ] AGENTS.md "Known Debt" no longer demands completing the already-complete
      Rust cutover; lifecycle prose references only commands/skills that
      exist (`/settle` is referenced but not installed — fix or drop).
- [ ] SLICE-1..4 docs and exemplars.md are either updated, moved to an
      archive location, or explicitly marked historical — none presented as
      active ground truth.
- [ ] One page documents the deployed surface for agents: app name, region,
      env contract (auth allowlist, store selection), and the smoke commands
      proven in QA.
- [ ] README quickstart commands run as written on a fresh clone.

## Notes

Drift found while grooming: AGENTS.md Known Debt predates cutover commit
fef534e; SLICE-*.md and exemplars.md describe the TS lift; CLAUDE.md was
already rewritten and is current. Per harness doctrine this is backlog work,
not ceremony — and it directly de-risks the next agent session in this repo.
