# Beta Content Generation

Refs-backlog: 27

## Purpose

`experiments/beta-generation/` is the beta content-generation seam. It turns
persisted source material into source-grounded quiz and exercise drafts, records
generation receipts, and keeps every generated artifact outside the published
`src/` kernel.

The default implementation is deliberately deterministic and live-provider-free
so CI can exercise the contract without network calls. Real provider calls must
plug in through the `LearningContentGenerator` interface and satisfy the same
source provenance, validation, and receipt rules.

## Workflow

1. Ingest source material into `BetaPersistenceStore` as `SourceDocument`
   records.
2. Compile source text into candidate learning activities. Structured fixture
   blocks are supported for precise tests; arbitrary prose can produce citeable
   fact drafts when it contains simple source-backed statements.
3. Create `ReferenceSpan` records for cited source evidence.
4. Save a `GenerationRun` receipt with provider/model/version metadata.
5. Save generated drafts with source ids, reference span ids, activity kind,
   activity stage, validation status, critique notes, and optional worked
   solution.
6. Approve only accepted drafts into review units consumed by the existing
   service, queue, and scheduling path.

## Fixture Format

The deterministic fixture path reads blank-line-separated blocks from a source
body.

```text
Concept: NATO CAT composition
Activity: exercise
Stage: composition
Question: Spell CAT over the phone using the NATO phonetic alphabet.
Answer: CHARLIE ALFA TANGO
Worked Solution: C is CHARLIE, A is ALFA, and T is TANGO.
Reference: C is CHARLIE. A is ALFA. T is TANGO.
```

Supported fields:

- `Concept`: concept identity used for queue/progression metadata.
- `Activity`: `quiz` or `exercise`.
- `Stage`: beta-owned ladder stage such as `recognition-3`, `free-recall`, or
  `composition`.
- `Question`: learner-facing prompt.
- `Answer`: accepted answer or expected solution.
- `Distractors`: comma-separated choices for multiple choice quizzes.
- `Worked Solution`: required for accepted exercises.
- `Reference`: cited evidence persisted as a `ReferenceSpan`.
- `Unsupported`: marks a candidate as rejected when deterministic fixtures need
  to model unsupported or unsafe generation.

## Validation Rules

The probe saves accepted and rejected drafts when provenance exists, and records
malformed candidates as generation-run failures when provenance is missing.

Accepted drafts require:

- persisted source document id;
- source reference span;
- question, answer, concept, activity kind, and stage;
- model/provider metadata;
- no duplicate-ish signature in the run or existing snapshot;
- no unsupported-generation marker;
- worked solution for exercises.

Rejected drafts remain useful evidence. They preserve source ids, reference
span ids, critique notes, and rejection reasons so later model-backed
generation can be evaluated rather than hand-waved.

Arbitrary prose that cannot produce source-backed drafts is recorded as a
generation failure instead of silently creating empty practice. That keeps the
phone UI honest: it asks for clearer source material rather than pretending a
weak input produced useful SRS.

## Kernel Boundary

No generation code, provider SDK, prompt template, vector index, source parser,
or persistence dependency was added under `src/`.

The kernel currently owns only stable learning behavior:

- canonical prompt shapes;
- grading envelopes;
- FSRS schedule state;
- progression metadata;
- queue metadata and selection;
- service attempts and applied-review outcomes.

The beta generation layer owns:

- activity kind and activity stage;
- distractor construction;
- worked solutions;
- generation run receipts;
- rejection/critique policy;
- source parsing and provenance.

The current arbitrary-prose compiler uses deterministic heuristics only as the
CI-safe stand-in for the future model-backed compiler. It is not the trust
boundary for real beta use; provider-backed generation still needs schema,
latency, cost, unsupported-claim, and citation receipts before dogfood trust.

## Eval Gaps

The deterministic path proves the storage, provenance, and validation contract.
It does not yet prove model quality.

Before model-backed generation can be trusted, add evals for:

- duplicate detection precision/recall;
- unsupported-claim rate;
- exercise solvability;
- worked-solution correctness;
- answer leakage in hints or feedback;
- stage calibration, such as whether a generated exercise is actually harder
  than a recognition quiz;
- latency and cost per generation run.

## Verification

```sh
bun test experiments/beta-generation/
bun run ci
```

The focused suite covers accepted quiz drafts, accepted exercise drafts,
promotion into review units, rejected unsupported drafts, duplicate-ish drafts,
and missing-provenance failures.
