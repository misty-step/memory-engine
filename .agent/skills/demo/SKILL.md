---
name: demo
description: |
  Produce memory-engine evidence for reviewers: command results, usage examples, fixture/dogfood receipts, and concise package-surface summaries. Trigger: /demo.
argument-hint: "[ticket|surface|PR]"
---

# /demo

This repo's demo is usually evidence that a library behavior can be consumed. Pick the smallest truthful format: command receipt, package import snippet, fixture output, dogfood receipt, beta receipt, or a concise PR evidence note.

For API changes, show imports from `memory-engine`, `memory-engine/testkit`, `memory-engine/adapters`, or the relevant modular subpath. For internal kernel changes, name the touched module and focused oracle. For beta/application-facing work, include the ticket-named dogfood or beta proof command and any doc receipt under `docs/dogfood/` or `docs/beta/`.

A demo answers: what moved, how a consumer calls it, what exact command proved it, and what remains unverified.

`bun run ci` IS the gate. It shells out to `dagger call check --source=.` and runs install, typecheck, Biome check, coverage-enforced tests, and Gitleaks.
