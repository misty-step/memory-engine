# Agent Operations

Guidance for any coding agent (Claude Code, Cursor, Aider, autopilot,
builder subagents) operating in this repo. Claude Code also reads
`CLAUDE.md`; read both.

## Before touching code

1. Read `SPEC.md` (strategy) and `SLICE-1-KERNEL.md` (current slice
   context packet).
2. Check `backlog.d/` for the ticket you are executing. If there is no
   ticket, stop and invoke `/shape` or `/groom` first — do not improvise
   features.
3. Read `exemplars.md` for the external patterns to lift.

## Feedback loop

`bun run ci` is the gate — typecheck, lint/format, test, in that order.
Run it locally before and after every change. If CI fails, fix the
underlying issue; never bypass (`--no-verify`, `|| true`, etc.).

Dagger parity: `dagger call ci --source=.` runs the same gate in a
container. Useful for reproducing CI failures locally.

## Change discipline

- One ticket → one branch → one PR. Branch from `master`.
- TDD default. Write the failing test first, commit it, then implement.
- Commit in small logical chunks. Each commit should leave CI green.
- Never add a runtime dependency without updating `SLICE-1-KERNEL.md`'s
  scope or shaping a follow-up ticket. `ts-fsrs` is the only dep slice 1
  is permitted.
- Never introduce `any`, non-null assertions (`!`), or `@ts-ignore`.
  Biome blocks them. If the types fight you, the model is wrong.

## The invariants (read CLAUDE.md for the full list)

Seven invariants govern this kernel. The three that trip agents most often:

1. **Core is pure** — no framework imports in `src/`.
2. **`ScheduleState` shape is ts-fsrs-native** — do not translate to camelCase.
3. **`Grader.grade()` returns one envelope** — rating is populated by an
   injected policy, not a second call.

## When blocked

- Two consecutive tool/test failures on the same thing → stop. Re-read the
  error and the current file in full. Do not open more files.
- Three edits to the same file in one session → stop. Plan the remaining
  changes and make one edit.
- If a spec and the code disagree, the spec wins until the spec is updated.
  Update the spec in a separate commit on the same PR.

## Delivery handoff

A ticket is done when:
- All oracles listed in the ticket / context packet pass.
- `bun run ci` exits 0.
- The Vault SRS canary (slice 1 only) is green if the ticket touches
  grading or scheduling.
- A PR description links the ticket file in `backlog.d/`.
