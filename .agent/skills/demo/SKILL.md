---
name: demo
description: |
  Produce memory-engine evidence for reviewers: command results, usage examples, fixture/canary receipts, and concise package-surface summaries. Trigger: /demo.
argument-hint: "[ticket|surface|PR]"
---

# /demo

This repo's demo is not a screenshot. It is evidence that a library behavior exists and can be consumed. Pick the smallest useful format: focused test output, `bun run ci` receipt, package import snippet, fixture corpus example, or Scry/Vault canary receipt.

For API changes, show a short TypeScript usage example importing from `memory-engine`, `memory-engine/testkit`, or `memory-engine/adapters`. For internal changes, name the touched module and oracle. For consumer-facing contract changes, include canary branch and command.

A good demo answers: what moved, how to call it, what command proved it, and what consumer path remains unverified if any.
