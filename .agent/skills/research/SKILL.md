---
name: research
description: |
  Research for memory-engine decisions: official docs, prior art, consumer exemplars, and multi-perspective validation before shaping shared kernel contracts. Trigger: /research.
argument-hint: "[query|docs|prior-art|consumer]"
---

# /research

Use research when a memory-engine decision depends on external facts, unfamiliar consumer behavior, or current library semantics. The result must feed `/shape`, `/groom`, `/code-review`, or a ticket oracle; it is not permission to add abstractions.

## Repo Anchors

Start with `.spellbook/repo-brief.md`, `SPEC.md`, `SLICE-*.md`, `exemplars.md`, `package.json`, `.dagger/src/index.ts`, and the active `backlog.d/` ticket. For beta/application pressure, read `docs/research/`, `docs/dogfood/`, `docs/beta/`, `experiments/`, and `service/` before looking outside.

Use primary sources for Bun, TypeScript, Biome, Dagger, and `ts-fsrs`. For learning-science decisions, prefer durable papers and the existing research notes under `docs/research/`. For model/provider behavior, keep SDK details out of `src/` and shape only adapter or beta-layer contracts.

## Output

Name the decision, sources, what changed about kernel scope, what stays application-owned, and the executable proof command. If the research cannot identify a stable shared contract, recommend a beta or dogfood probe instead of a package change.

`bun run ci` IS the gate. It shells out to `dagger call check --source=.` and runs install, typecheck, Biome check, coverage-enforced tests, and Gitleaks.
