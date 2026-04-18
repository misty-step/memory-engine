---
shaping: true
ticket: 13-vault-rubric-canary
slice: 3
status: ready
priority: high
estimate: M
depends_on: [11-rubric-grading-contract, 12-adapter-surface]
oracles:
  - bun run ci
  - (cd /Users/phaedrus/Documents/daybook/tools/vault-srs && git switch memory-engine-rubric-canary && bun test)
---

# Vault rubric canary — slice-3 acceptance gate

## Goal

Replace Vault's local rubric contract plumbing with memory-engine's shared
rubric prompt/result types and adapter interface, while keeping Vault's existing
LLM implementation and tests green.

## Non-Goals

- No prompt-engineering migration into memory-engine.
- No deterministic-regression rewrite. Slice-1 deterministic imports stay as-is.
- No Vault schema migration.
- No vendor lock-in inside core.

## Oracle

- [ ] `cd /Users/phaedrus/Documents/daybook/tools/vault-srs && git switch
      memory-engine-rubric-canary && bun test` exits 0.
- [ ] Vault's local rubric grader implements the shared memory-engine adapter
      interface rather than an app-local contract.
- [ ] Vault deterministic grading still imports from `memory-engine` and stays
      green.
- [ ] `bun run ci` in `memory-engine` still exits 0 after canary-driven kernel
      changes.

## Notes

- Keep Vault's actual OpenAI request/prompt builder local. The canary is about
  contract reuse, not vendor abstraction theater.
- If the canary exposes missing fields or ergonomics problems, fix the kernel
  contract first rather than patching around them in Vault.
- Study:
  - `/Users/phaedrus/Documents/daybook/tools/vault-srs/src/grading.ts`
  - `/Users/phaedrus/Documents/daybook/tools/vault-srs/src/rubric-grader.ts`
  - `/Users/phaedrus/Documents/daybook/tools/vault-srs/tests/grading.test.ts`
