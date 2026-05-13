---
name: ci
description: |
  Run and diagnose memory-engine's canonical CI gate. Use when asked to run CI, check gates, fix a red pipeline, verify a change, or explain CI failures. Trigger: /ci, /gates.
argument-hint: "[--local|--full]"
---

# /ci

Run the correctness gate for `memory-engine`, a pure TypeScript learning kernel. CI confidence here means the package typechecks, Biome accepts lint and format, coverage-enforced Bun tests pass, and Dagger runs the secret scan.

`bun run ci` IS the gate. It shells out to `dagger call check --source=.` and runs install, typecheck, Biome check, coverage-enforced tests, and Gitleaks.

## Modes

- Default / `--full`: run `bun run ci`.
- `--local`: run `bun run ci:local` as an inner loop only.

`bun run ci:local` runs `bun run typecheck && bun run check && bun run coverage`. Use it while iterating, but never present it as delivery evidence. Delivery requires `bun run ci`.

## Process

1. Confirm the work is tied to an active `backlog.d/` ticket unless the request is read-only diagnosis.
2. Use focused commands while iterating: `bun run typecheck`, `bun run check`, `bun run coverage`, or a targeted `bun test tests/...`.
3. Before handoff, run `bun run ci`.
4. If Dagger fails, classify the failing lane: typecheck, lint, coverage, or secrets. Fix the underlying issue; never bypass the gate, lower coverage, weaken Biome, add `any`, add non-null assertions, or use `@ts-ignore`.
5. Re-run the narrow failing command, then finish with `bun run ci`.

## Boundaries

Do not deploy from this skill; this repo has no deploy target. Do not run consumer canaries unless the active ticket names them. Scry and Vault SRS canaries are product proof, reported separately from CI.

## Output

Report the exact command, final status, gates covered, and residual unverified paths. A green report must name `bun run ci`; a local-only report must say it is inner-loop evidence only.
