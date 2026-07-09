# Beta Content Generation

Refs-Powder: memory-engine-027
Refs-Powder: memory-engine-051
Refs-Powder: memory-engine-052

## Purpose

`crates/memory-engine-generation` is the deterministic content-generation
probe for the beta interface. It turns persisted source material into
source-grounded quiz and exercise drafts, records generation receipts, and
keeps every generated artifact outside the published kernel.

This is deliberately not a model-provider integration. It is the contract and
QA spine that real provider calls must satisfy later.

## Workflow

1. Ingest source material into `BetaPersistenceStore` as `SourceDocument`
   records. User-facing app flows accept one free-form capture field; title is
   inferred from the first meaningful source text and may be edited in later
   source-management work, but it is not required up front.
2. Classify the source learning intent as one of `verbatim memorization`,
   `concept understanding`, `fact recall`, or `procedure/process`.
3. Branch generation by intent before creating candidate learning activities:
   verbatim sources produce recitation-ladder exercises, concept sources
   produce explanation/application prompts, fact sources produce recognition
   and recall checks, and process sources produce ordered-step prompts.
4. Parse deterministic source blocks into candidate learning activities when
   the source is already written in fixture format.
5. Create `ReferenceSpan` records for cited source evidence.
6. Save a `GenerationRun` receipt with provider/model/version metadata. Bridge
   generation runs also persist the parent `ReviewUnitId` so QA and later
   analysis can trace generated scaffold items back to the failed item.
7. Save generated drafts with source ids and/or concept reference note keys,
   reference span ids when source evidence exists, activity kind, activity
   stage, validation status, critique notes, and optional worked solution.
8. Approve only accepted drafts into review units consumed by the existing
   service, queue, and scheduling path.
9. For review escape hatches, cache provider-written `ConceptReferenceNote`
   records by concept key. Source-backed reference views use `ReferenceSpan`
   text first; spanless items generate the concept note once and reuse it on
   later views. Bridge material cites that note and creates lower-stage queue
   items while the parent is deferred.

## Capture Contract

The beta app and production app shell expose a single capture affordance named
`capture`. It accepts anything from one word to a pasted article. Existing
machine clients may still send `title` and `body`, but learner-facing forms do
not split the task into title/body fields or reveal structured fixture syntax.
When the title is absent, the study boundary infers a short title from the first
meaningful sentence or line and stores the capture as an ordinary text
`SourceDocument`. App posts save that source first and render the saved material
list with a separate review-creation action, so learners can leave and return
to the captured source even if generation is slow or fails.

Image capture is scoped out for this ticket. OCR introduces a second provider
with different latency, cost, privacy, and quality failure modes; accepting
pasted text keeps 051 focused on the capture and intent-generation contract.
The call changes when there is an OCR boundary with corpus fixtures for image
quality, cost ceilings, and human-readable failure notices.

## Fixture Format

The deterministic probe reads blank-line-separated blocks from a source body.

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

Accepted provider-generated source-backed drafts require:

- persisted source document id;
- source reference span;
- question, answer, concept, activity kind, and stage;
- model/provider metadata;
- no duplicate-ish signature in the run or existing snapshot;
- no unsupported-generation marker;
- worked solution for exercises.

Accepted bridge drafts require:

- a persisted concept reference note key;
- no source document ids unless they also cite source reference spans;
- question, answer, concept, activity kind, and lower activity stage than the
  parent item;
- model/provider metadata;
- no duplicate-ish signature against existing items or sibling bridge drafts;
- worked solution for bridge exercises.

Concept-note-backed bridge drafts are intentionally source-less. They cite the
cached note instead of pretending to have source spans. Provider-generated
source drafts that have source ids but no reference spans are rejected before
promotion.

Multiple accepted drafts may share the same `Concept` and `Stage`. The study
boundary treats those as item variants over one concept: each variant keeps its
own `reviewUnitId`, attempt trail, and schedule state, while due queue
selection rotates among same-concept same-stage siblings before showing the
same phrasing again.

Source generation rejects duplicate-ish accepted material by concept, answer,
and cheap normalized question-token similarity. That catches exact copies and
near-copy wording without blocking legitimate same-concept same-stage variants
that ask meaningfully different questions.

Rejected drafts remain useful evidence. They preserve source ids, reference
span ids or concept reference note keys, critique notes, and rejection reasons
so later model-backed generation can be evaluated rather than hand-waved.
When any first-pass draft for a source is rejected and the provider supports
repair, the runner makes one bounded repair request with the rejection reasons,
persists the repaired candidates through the same trust gate, and aggregates
the repair pass into the run-level usage totals. The repair pass is still
cost-capped: only the first bounded set of rejection reasons is sent.

## Kernel Boundary

No generation code, provider SDK, prompt template, vector index, source parser,
or persistence dependency was added under `crates/memory-engine-core`.

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
- concept reference notes and bridge-material validation.

## Eval Gaps

The deterministic probe proves the storage and validation contract. It does not
yet prove model quality.

Before model-backed generation can be trusted, add evals for:

- duplicate detection precision/recall;
- unsupported-claim rate;
- exercise solvability;
- worked-solution correctness;
- answer leakage in hints or feedback;
- stage calibration, such as whether a generated exercise is actually harder
  than a recognition quiz;
- bridge material quality: easier than the parent, faithful to the same
  concept, and non-duplicate against existing review items;
- latency and cost per generation run.

## Verification

```sh
cargo test -p memory-engine-generation
cargo test -p memory-engine-openrouter
cargo test -p memory-engine-study bridge_material_creates_easier_due_items_before_the_parent
cargo test -p memory-engine-bench bridge_quality_scenario_requires_easier_faithful_non_duplicate_items
bun run ci
```

The focused suite covers accepted quiz drafts, accepted exercise drafts,
promotion into review units, rejected unsupported drafts, duplicate-ish drafts,
missing-provenance failures, concept-note fallback, and bridge material.
`memory-engine-openrouter` contract tests cover reference-note payloads and the
bridge-material prompt/schema mapping, including the two-step scaffold ladder
(`recognition-bridge` then `cued-recall-bridge`).

The former TypeScript `experiments/beta-generation/` runtime oracle was deleted
after the Rust crate covered deterministic generation and fixture parity.
