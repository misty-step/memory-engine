# Backlog

`backlog/` is the sole active work ledger. One markdown file per item.
This index is the board.

Status values: `ready`, `design`, `proof`, `later`.
Priority values: `p0`, `p1`, `p2`, `p3`.

A `ready` item has executable acceptance and proof. Claim it by setting
`status: in-progress` in the file and listing it under In progress here.
A `design` item is not claimable for implementation until the open
questions are locked. A `proof` item is merged; production evidence remains.
`later` is parked.

Each item file uses these headings: Outcome, Why now, Acceptance, Dependencies,
Proof, Non-goals.

Commits and pull requests use `Refs backlog/<id>`. Use `Closes backlog/<id>`
only when the merge satisfies every acceptance criterion and no post-merge
proof remains.

## In progress

None.

## Ready

- [119 Instant keep, skip, snooze, and hatches](119-instant-actions.md) — p0
- [121 Capture more opens create](121-capture-more.md) — p1
- [122 Edit drafts including distractors](122-edit-distractors.md) — p1

## Proof

Shipped on production `391fb55`. Operator walk remains.

- [117 Stay signed in on the phone](117-stay-signed-in.md) — p0
- [118 Gemini 3.7 Flash](118-gemini-37.md) — p0
- [120 Shuffle MCQ order](120-shuffle-mcq.md) — p1

## Design

- [123 Tighten the post-grade card](123-graded-card.md) — p1
- [124 Punch out to durable references](124-references.md) — p1
- [125 Snooze-concept grain and Bridge quality](125-overflow.md) — p1

## Later

Parked platform work. Do not claim while a ready daily-friction item exists.
See [later.md](later.md).
