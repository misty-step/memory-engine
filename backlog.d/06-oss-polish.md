---
shaping: true
ticket: 06-oss-polish
slice: 1
status: ready
priority: medium
estimate: S
depends_on: []
oracles:
  - bun run ci
  - gh repo view misty-step/memory-engine --json repositoryTopics,description
---

# Public repo polish — README + GitHub metadata

## Goal

Make `misty-step/memory-engine` legible to a stranger who lands on the
repo cold. Today the README reads like a private scratchpad (relative
paths to `../ruminatio`, no install, no usage, no scope framing), which
is fine for a private repo and embarrassing for a public one.

## Non-Goals

- No CONTRIBUTING.md, CODE_OF_CONDUCT.md, or issue templates until there
  is external contributor interest. Premature governance is overhead.
- No logo, hero image, or marketing copy.
- No npm publish (slice 2+).
- No changes to `SPEC.md`, `SLICE-*.md`, `exemplars.md`, `CLAUDE.md`,
  `AGENTS.md` — those are operator docs, not reader docs, and they're
  already correctly scoped.
- No blog post / announcement. Ship the polish, don't ship an event.

## Oracle

- [ ] `README.md` opens with a one-sentence "what this is" that a
      TypeScript developer unfamiliar with spaced repetition can parse.
- [ ] `README.md` contains: install snippet (how to consume from another
      project via local path / workspace — npm publish is out of scope),
      minimal usage example (one `next()` call, one `grade()` call),
      status/roadmap pointer (links to `SPEC.md` + slice docs), license
      line.
- [ ] `README.md` contains zero filesystem paths that only exist on
      Phaedrus's laptop (no `../ruminatio`, no
      `/Users/phaedrus/Documents/...`). Consumer apps are named and
      linked, not filesystem-pathed.
- [ ] GitHub repo has topics set: at minimum `spaced-repetition`,
      `fsrs`, `typescript`, `learning`. Verified by
      `gh repo view misty-step/memory-engine --json repositoryTopics`.
- [ ] GitHub repo homepage URL is either unset or points somewhere
      real (not a stale path).
- [ ] CI badge in README pointing at the GitHub Actions workflow.
- [ ] `bun run ci` exits 0 (no code changes expected, but verify the
      gate still runs).

## Notes

- The current README's four-bullet "Shared learning and memorization
  engine for" list is the right instinct but wrong form for a public
  audience. Reframe as: "extracted from four apps — links where public,
  names where private."
- Keep it short. A good kernel README is ~100 lines, not 500. Detail
  belongs in `SPEC.md`, which the README links to.
- Don't invent API examples — use real signatures from `src/index.ts`
  and `testkit/index.ts`. If the usage example would require code that
  doesn't exist yet (e.g. `Grader.grade` pre-ticket-03), either defer
  that section or note it as "coming in slice 1."
- Topics: GitHub allows up to 20. Don't spam — 4-6 well-chosen topics
  surface better in search than a grab-bag.
- This ticket is independent of 02/03/04/05. Can ship any time; a good
  candidate for a short session between kernel tickets.
