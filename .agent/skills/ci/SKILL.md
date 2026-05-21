---
name: ci
description: |
  Run and diagnose memory-engine's canonical CI gate. Use when asked to run CI, check gates, fix a red pipeline, verify a change, or explain CI failures. Trigger: /ci, /gates.
argument-hint: "[--local|--full]"
---

# /ci

Run the correctness gate for `memory-engine`, a pure TypeScript/Bun learning kernel. CI confidence means the package installs from lockfile, typechecks, passes Biome check, passes coverage-enforced Bun tests, and clears Gitleaks inside Dagger.

`bun run ci` IS the gate. It shells out to `dagger call check --source=.` and runs install, typecheck, Biome check, coverage-enforced tests, and Gitleaks.

## Modes

- Default / `--full`: run `bun run ci`.
- `--local`: run `bun run ci:local` as an inner loop only.

`bun run ci:local` runs `bun run typecheck && bun run check && bun run coverage`. It is not delivery evidence.

## Diagnosis

Classify failures by lane: install/lockfile, typecheck, Biome, coverage/test behavior, or secrets. Use focused commands while iterating, then finish with `bun run ci`. Never lower coverage, weaken Biome, add `any`, add non-null assertions, use `@ts-ignore`, bypass Gitleaks, or replace Dagger with adjacent evidence.

Historical Scry/Vault canaries are deprecated and are not part of CI proof.
