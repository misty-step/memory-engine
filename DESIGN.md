# Memory Engine Design System — "Ledger"

Status: the binding visual and interaction contract for every production
surface rendered by `crates/memory-engine-api-render`. Locked by operator
verdict 2026-07-09 (design lab LAB-001, option TASTE-1 "Ledger"; runners-up
TASTE-3, TASTE-2, APPLE-3). Agents doing visual work read this file first;
the conformance gate in `crates/memory-engine-api-render/src/design_preview.rs`
enforces the checkable parts.

Provenance note: Ledger deliberately diverges from the vendored Misty Step
aesthetic kit law (one size, zero radius, status-on-glyph-only). memory-engine
owns its system; `assets/ledger.css` is the single stylesheet of record.

## Character

Warm paper and ink. A memory instrument: calm, precise, trustworthy, quietly
confident. Sessions are 1 to 10 minutes, phone-first, often in stolen moments.
The review card is the whole product while it is on screen; everything else
gets out of its way.

## Tokens (`assets/ledger.css` `:root`, `--lg-*`)

Light ("paper"):

| token | value | role |
|---|---|---|
| `--lg-paper` | `#F6F2EA` | page ground |
| `--lg-paper-2` | `#EFE9DD` | raised surfaces: choices, fields, sheets |
| `--lg-ink` | `#1B1A16` | primary text, contained buttons |
| `--lg-ink-2` | `#57534A` | secondary text, labels |
| `--lg-line` | `#D9D1C1` | hairlines, borders |
| `--lg-accent` | `#B24E27` | the one signal: primary action, kickers |
| `--lg-accent-ink` | `#8E3B1B` | accent-on-paper text (AA) |
| `--lg-pine` | `#2F6146` | verdict Correct |
| `--lg-clay` | `#A6382C` | verdict Try again |
| `--lg-ochre` | `#9A6A16` | verdict Close, watch states |
| `--lg-slate` | `#4A5560` | verdict Revealed, neutral status |

Dark ("slate ground"): `--lg-ground #15130D`, paper `#1E1B13`, ink `#ECE6D8`,
ink-2 `#A79E8B`, line `#37301F`, accent `#E07B45`, pine `#4E9A72`, clay
`#D2604F`, ochre `#C79433`. Dark mode follows `prefers-color-scheme` via a
custom-property swap; both modes must hold WCAG AA.

Type: `--lg-sans: "Geist", …` for prose and controls; `--lg-mono: "Geist
Mono", …` for labels, counts, horizons, and every numeral. Numerals are
always mono + `tabular-nums` so counts align like a ledger.

Registers (exactly these; no ad-hoc sizes):

| register | spec | use |
|---|---|---|
| display | 30px+ / 640+ / -2% tracking | cover H1, workspace "Today", due hero count |
| prompt | 23px / 600 / 1.28 | the review question only |
| body | 16px / 400-600 / 1.45 | prose, choices, fields |
| meta | 13px / 500 | secondary prose, hints |
| label | 10-11px mono / 600 / letterspaced caps | kickers, group labels, ledger keys |

Space scale: 4 / 8 / 12 / 20 / 32. Radius: `--lg-r: 7px` (controls, cards),
`--lg-r-lg: 12px` (hero panels). Motion: `--lg-ease: cubic-bezier(.16,1,.3,1)`,
tap settle 190ms (1px translateY on `:active`). Motion is feedback only; the
sole looping element is the generating pulse on an in-flight job, and it
stops when the job resolves. `prefers-reduced-motion` removes translates and
the pulse; state color applies instantly.

## Interaction law (every click fights for its life)

- MCQ: **one tap answers.** The choice buttons are the submit; there is no
  separate confirm.
- Free response: type, then one submit.
- Graded Correct and Graded Close / Try again / Revealed: **no auto-advance,
  ever.** Verdict prints in place (correct row tinted pine, horizon on the
  row), meta ledger below, and the page holds indefinitely: the learner
  reviews the verdict, answer key, and dossier for as long as they want. Only
  a deliberate Continue tap (or Enter while it is focused) advances the card
  — incidental taps while reading must never advance it. Operator ruling
  from live dogfood use (memory-engine-081) reverses the two-speed advance
  shipped in memory-engine-078: it is dead law, correct or not.
- Pre-grade shows **no card meta**: no stage, no last-seen, no success rate,
  no health. Just kicker, prompt, the answer mechanism, and the hatch row.
- Escape hatches: only **Reveal answer** stays on the card, beside one `···`
  (More) disclosure holding Reference, Skip, Snooze, Bridge, Delete, and the
  Capture punch-out. Six permanent buttons under a card is a defect. Every
  action in the disclosure carries a leading icon and a tooltip truthful to
  what the route actually does (Skip defers within the session; Snooze
  defers until tomorrow — they must never read as interchangeable).
- The workspace's first element is the due hero: count + one Start review tap.

## Post-grade meta ledger

After grading (and only after), the card shows its record: verdict + revealed
answer, "you'll see this again …" horizon, then a mono ledger of stage,
last seen, success rate (n/N and trend), and the concept line. This is the
learner's honest dossier on the card; it never appears pre-grade.

## Component grammar

- **Choices** (`lg-choice`): full-width rows on `--lg-paper-2`, 1px `--lg-line`
  border, radius 7, min-height 56px, mono key chip. Graded: correct row pine
  wash (`color-mix` ~14%) + pine border + horizon; chosen-wrong row clay wash;
  others dim to 42%. Never a left-only border stripe.
- **Buttons**: contained ink (`lg-btn`) for primary; accent fill only for
  Start review; quiet outline (`lg-quiet`) for hatches. All tap targets are
  at least 44px.
- **Fields**: bounded boxes on `--lg-paper-2`, radius 7; focus = 2px accent
  outline, offset 1.
- **Due hero**: ink-bordered radius-12 panel; mono display count; accent
  Start button.
- **Feed rows / health rows**: hairline-topped rows, mono meta right-aligned;
  the health meter is a 4px bar tinted pine/ochre/clay by health, with the
  mono stat line beneath.
- **Cover**: display H1, mono tagline, email + send stacked full-width;
  "No passwords. Ever." mono footline.
- **Layout**: single column, measure-width, centered; the page scrolls
  vertically, never horizontally. Any horizontal scrollbar at 390px is a
  release-blocking defect.

## Anti-patterns (reject on sight)

Left-border accent stripes on cards. Pre-grade meta of any kind. Any form of
auto-advance on a graded page, correct or not. Gradient text, glass,
blobs, purple-on-black. Ambient motion. Em-dashes in UI copy. New raw hex
values outside the `ledger.css` token block. Fabricated numbers (counts,
times, rates come from real state — the honest-effort invariant,
memory-engine-074).

## Enforcement

- `design_preview.rs` conformance tests assert the Ledger law: token sheet
  present, register scale exact, verdict tint classes, hatch collapse,
  pre-grade/post-grade meta split, no raw hex outside tokens in `render.rs`.
- Behavior tests in `memory-engine-api` assert the interaction law at the
  route boundary (one-tap MCQ, meta split, Continue as the only advance).
- The live phone walk (390×844) is the overflow gate: no horizontal scroll on
  cover, workspace, review, graded, and sheet-open states.
