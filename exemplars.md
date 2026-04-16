# Exemplars

External patterns this repo lifts or follows. Study these files before
implementing the corresponding feature. Copy the semantics; only rename
where `SLICE-1-KERNEL.md` documents a rename.

## Deterministic grading (primary exemplar)

**`/Users/phaedrus/Documents/daybook/tools/vault-srs/src/grading.ts`**

The reference implementation for slice-1's `Grader`. Lift verbatim:

- Levenshtein distance thresholds by answer length (`0` for ≤4 chars,
  `1` for ≤8, `2` for >8).
- Accepted-variant substitution.
- Equivalence group handling.
- Ignored-token stripping.
- The `GradeResult` envelope shape (modulo the documented rename
  `outcome` → `verdict`).

Treat divergence from these semantics as a bug until slice 2.

## Grading fixtures

**`/Users/phaedrus/Documents/daybook/tools/vault-srs/tests/grading.test.ts`**

Fixture corpus that slice-1's parity oracle must reproduce exactly. The
fixtures ship to `testkit/fixtures.ts` and become part of the consumer
contract test suite.

## Canonical type shapes

**`/Users/phaedrus/Documents/daybook/tools/vault-srs/src/types.ts`**

Strongest existing canonical type shape in the portfolio. `GradeResult`,
`ItemType`, and `FsrsCardState` are reference shapes for slice 1.
Progression fields (`progressionGroup`, `requires`, `supersedes`,
`stageOrder`) are out of scope for slice 1 — note them for slice 2.

## Rating policy (default)

**`/Users/phaedrus/Development/ruminatio/convex/lib/grading.ts`**

The default `ratingPolicy` function lifts Ruminatio's verdict-to-rating
logic verbatim:

- `correct` + `responseTimeMs ≤ 4000` + `priorReps ≥ 3` → `4` (Easy)
- `correct` otherwise → `3` (Good)
- `close` → `2` (Hard)
- `wrong | revealed` → `1` (Again)

Slice 1 ships this as the sole policy; alternative policies are a slice-2
concern.

## FSRS wrapper shape

**`/Users/phaedrus/Development/caesar-in-a-year/lib/srs/fsrs.ts`**

Simplest FSRS wrapper in the portfolio. Confirms that the kernel's
single-function scheduler surface (`next(state, rating, now) → state`)
is sufficient. Reference for the wrapper's compactness, not the rating
mapping (which comes from Ruminatio instead).

## Concept-vs-phrasing boundary

**`/Users/phaedrus/Development/scry/convex/fsrs/engine.ts`**

Validates that concept-level consumers can sit on the same `ScheduleState`
as phrasing-level consumers. The key is the opacity of `ReviewUnitId` —
the kernel never inspects what a review unit represents. Read this when
tempted to add structure to `ReviewUnitId`.

## What to ignore

- **Session builders** (`caesar-in-a-year/lib/session/builder.ts`,
  Ruminatio's session flow). These are app-specific choreography and must
  stay out of the kernel.
- **Phrasing generation** (`scry/convex/phrasings.ts`). App-specific
  content pipeline.
- **Content parsers** (`vault-srs/src/parser.ts`, Ruminatio's markdown
  import). Authoring formats belong to apps, not the kernel.
