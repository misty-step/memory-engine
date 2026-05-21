---
name: diagnose
description: |
  Diagnose memory-engine failures from exact oracles: Dagger lanes, Bun tests, typecheck, Biome, coverage, Gitleaks, dogfood lanes, or current proof commands. Trigger: /diagnose.
argument-hint: "[symptom|command|proof]"
---

# /diagnose

Start from a reproduced failure, not a theory. Re-run or inspect the exact failing command: `bun run ci`, `bun run qa`, a focused `bun test`, typecheck, Biome, coverage, Gitleaks, package export smoke, dogfood lane, beta test, or ticket-named external proof.

After two failures on the same command, stop and read the error plus the current file(s) in full before opening more surfaces.

Classify by surface: install/lockfile, type system, Biome, coverage/test behavior, secret scan, package export, scheduler/grader/progression/queue contract, adapter/testkit contract, service prototype, dogfood path, beta proof, or harness lifecycle. Fix the root cause; do not lower gates.

For application-facing proof, fix the kernel only when the shared contract is wrong. If the beta or consumer owns the behavior, record or shape that issue outside `src/`.

`bun run ci` IS the gate. It shells out to `dagger call check --source=.` and runs install, typecheck, Biome check, coverage-enforced tests, and Gitleaks.
