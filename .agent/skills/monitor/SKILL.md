---
name: monitor
description: |
  Watch memory-engine signals after CI, QA, release prep, or repeated workflow. This repo has no production release surface; monitor CI failures, QA regressions, coverage drift, package export breakage, dogfood/beta evidence, benchmark drift, and backlog lifecycle contradictions. Trigger: /monitor.
argument-hint: "[signal|--grace duration]"
---

# /monitor

`memory-engine` has no deploy target. Monitoring here means watching repository signals that prove the package and beta dogfood loop remain healthy: CI, QA, coverage, package exports, dogfood lanes, beta persistence/generation receipts, benchmark drift, and backlog lifecycle contradictions.

Do not invent production telemetry, healthcheck URLs, rollout watches, or rollback steps. If a future ticket adds a real release surface, tailor a release skill and update this monitor path then.

## Signal Paths

- Canonical gate: `bun run ci` and GitHub Actions running `dagger call check --source=.`.
- QA sweep: `bun run qa`, including dogfood experiments and benchmarks.
- Focused beta lanes: `bun test experiments/beta-store/`, `bun test experiments/beta-generation/`, future `experiments/beta-study/` tests.
- Package surface drift: `tests/api/`, `tests/testkit/`, `tests/adapters/`, and `package.json` exports.
- Backlog lifecycle drift: active `backlog.d/` tickets whose closure trailers or oracles indicate they should be archived.
- Documentation drift: `docs/dogfood/`, `docs/beta/`, and `docs/qa/` receipts falling behind executable behavior.

## Action

Observe and escalate. If a signal trips, hand off to `/diagnose` with the exact command, artifact, or contradiction. Do not perform root-cause repair in `/monitor`.

`bun run ci` IS the gate. It shells out to `dagger call check --source=.` and runs install, typecheck, Biome check, coverage-enforced tests, and Gitleaks.
