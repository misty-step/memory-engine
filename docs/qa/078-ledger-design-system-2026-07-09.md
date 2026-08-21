# QA 078 — Ledger design system ship (2026-07-09)

Legacy work item: `memory-engine-078`. Operator verdict (design lab LAB-001,
round 1) locked TASTE-1 "Ledger" as the memory-engine design system, with
rulings: pre-grade card minimal, card meta post-grade only, no left-border
cards, no horizontal scroll, every click justified or eliminated. `DESIGN.md`
at the repo root is the binding contract.

## What shipped

- `crates/memory-engine-api-render/assets/ledger.css` — the repo-owned
  system: warm paper/ink tokens (light + dark via `prefers-color-scheme`),
  Geist/Geist Mono registers, mono tabular numerals, verdict marks (pine /
  clay / ochre / slate), 7/12px radii, 190ms tap settle, reduced-motion
  reset. Replaces the vendored aesthetic kit on this surface (provenance
  divergence recorded in DESIGN.md).
- Review recomposition: pre-grade = prompt + answer mechanism + Reveal + one
  `···` More disclosure (Reference / Skip / Snooze / Bridge / Delete +
  Capture punch-out). Post-grade = verdict + revealed answer + horizon +
  the dossier (`me-meta-ledger`: stage, last seen, success record, concept
  line) + Continue.
- Two-speed advance: graded-correct carries `data-auto-advance="2000"`;
  `app.js` submits the Continue form after the hold (tap or Enter anywhere
  advances sooner). Misses hold for study. JS-off always has Continue.
- UI copy: em-dashes removed per the Ledger anti-pattern law.
- Conformance gate rewritten (`design_preview.rs`): Ledger stylesheet link,
  no inline styles beyond data-driven meter widths, no raw hex in markup, no
  left borders, hatch-row class dead, dossier post-grade only, auto-advance
  split asserted on the preview states.

## Live phone walk (iPhone 12, 390×844)

Local api (file store, debug links) on `127.0.0.1:8476`; agent-browser
isolated session. `document.scrollWidth == innerWidth == 390` (no horizontal
scroll) verified on every screen below.

| state | evidence | overflow |
|---|---|---|
| Signed-out cover | `078-cover-phone.png` | 390 vs 390 |
| Workspace, empty | `078-workspace-empty-phone.png` | 390 vs 390 |
| Workspace, 2 due (due hero) | `078-workspace-due-phone.png` | 390 vs 390 |
| Review pre-grade (free response) | `078-review-pregrade-phone.png` | 390 vs 390 |
| More disclosure open | `078-review-more-sheet-phone.png` | — |
| Graded miss + dossier | `078-graded-wrong-dossier-phone.png` | 390 vs 390 |
| Graded correct + dossier | `078-graded-correct-phone.png` | — |
| Workspace, dark mode | `078-workspace-dark-phone.png` | 390 vs 390 |

Interaction law verified live:

- Wrong answer (`DELTA OSCAR GOLF` on the CAT card): verdict "Try again" in
  clay, revealed answer, dossier rendered, and the page **held** through a
  2.6s wait — no auto-advance on a miss.
- Correct answer (`CHARLIE ALFA TANGO`): verdict "Correct" in pine with the
  drawn check, dossier rendered, and the page **auto-advanced to
  `/app/next` with zero additional input** after the hold.
- MCQ remains one tap (choice buttons are the submit; no confirm step).
- Graceful stale-submit recovery observed: a duplicate submit from a stale
  page lands back on the workspace with a quiet "Review unit not found."
  notice rather than an error page.

## Fixes found by the walk (shipped in this change)

- More-sheet grid needed `display: contents` on its forms so hatch buttons
  fill the grid evenly.
- Dossier SUCCESS row duplicated the count (`success_rate` already carries
  it); now `{success_rate} · {trend}`.
- Em-dash copy across cover/workspace/jobs violated the new law; rewritten.

## Gates

- `bun run ci` (fmt, workspace tests incl. rewritten conformance, clippy
  `-D warnings`, rustdoc): green.
- `bun run ci:full` and hosted CI: recorded with the ship evidence.

## Residual

- Free-response answers keep one submit button tap (typing means the hands
  are already on the keys; Enter submits with JS).
- The auto-advance hold is a fixed 2000ms; a per-user or
  length-aware hold is future tuning, and WCAG timing concerns are mitigated
  by the miss-holds rule, the tap-to-advance override, and the JS-off
  Continue path.
- Concept-level snooze and card editing are carded separately
  (memory-engine-079, memory-engine-080); the More sheet has their slots.
