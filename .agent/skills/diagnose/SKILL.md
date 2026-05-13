---
name: diagnose
description: |
  Diagnose memory-engine failures from exact oracles: Dagger lanes, Bun tests, typecheck, Biome, coverage, Gitleaks, or consumer canaries. Trigger: /diagnose.
argument-hint: "[symptom|command|canary]"
---

# /diagnose

Start from a reproduced failure, not a theory. Re-run or inspect the exact failing command: `bun run ci`, a Dagger lane, a focused `bun test`, or the named Scry/Vault canary. After two failures on the same thing, stop and read the error and current file in full before opening more files.

Classify failures by surface: type system, Biome, coverage/test behavior, secret scan, package export, scheduler/grader/progression/queue contract, adapter contract, or external consumer canary. Fix the root cause; never lower a gate.

For canaries, remember that consumers own persistence and mapping. Fix the kernel only when the shared contract is wrong; otherwise record the consumer-side issue. Finish with the exact command that proves the fix, usually `bun run ci` plus the failing canary.
