---
name: research
description: |
  Research for memory-engine decisions: official docs, prior art, consumer exemplars, and multi-perspective validation before shaping shared kernel contracts. Trigger: /research.
argument-hint: "[query|docs|prior-art|consumer]"
---

# /research

Use research when a design decision depends on facts outside the current repo or on unfamiliar beta/application behavior. Prefer primary sources: official TypeScript, Bun, Dagger, Biome, and `ts-fsrs` docs; current beta/dogfood code; active application code named by the ticket; and existing `exemplars.md`.

Research output must feed a shaped ticket or context packet. Summarize what evidence changes about kernel scope, application ownership, or proof design. Do not use research as permission to add abstractions; use it to decide the smallest stable contract.

For changing OpenAI or other vendor behavior, keep SDK details out of core and shape adapter contracts only.
