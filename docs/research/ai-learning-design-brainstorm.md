# AI Learning Design Brainstorm

This brainstorm turns AI/embedding/agent research into concrete experiments and
future contracts. The bias is: build AI-heavy clients around a pure learning
kernel, not AI glue inside the kernel.

Refs-Powder: memory-engine-020
Refs-Powder: memory-engine-021
Refs-Powder: memory-engine-022
Refs-Powder: memory-engine-023
Refs-Powder: memory-engine-024

## North-Star Ideas

### 1. Learning Compiler

An agent ingests notes, PDFs, transcripts, markdown, or pasted text and emits a
reviewable learning graph:

- canonical prompts
- progression stages
- prerequisites
- distractors
- rubrics
- source citations
- rejected product-owned fields

Embeddings cluster candidate concepts and detect duplicates. A model proposes
the graph. Deterministic validators reject missing IDs, unsupported source
claims, duplicate units, or progression cycles.

Kernel pressure: stable prompt/progression/queue/testkit contracts.

Client-owned: parsing, embeddings, prompts, citations, source files.

### 2. Misconception Graph

Wrong answers become diagnostic data. The client retrieves similar historical
wrong answers, known misconceptions, and prerequisite concepts, then proposes a
repair path:

- "you confused X with Y"
- "review prerequisite Z"
- "try contrastive item A/B"
- "generate self-explanation before next reveal"

Kernel pressure: progression edges, attempt metadata, eval fixtures.

Client-owned: diagnosis model, misconception taxonomy, repair prompt copy.

### 3. Contrastive Queue Planner

Embeddings enrich queue candidates with similarity neighborhoods. The queue can
interleave related-but-confusable items instead of random domains.

Examples:

- Latin active vs passive verb endings
- similar prayer clauses
- theorem hypotheses with near-miss conditions
- vocabulary false friends

Kernel pressure: maybe similarity tags or client-provided grouping metadata.

Client-owned: embedding model, thresholds, vector index, similarity policy.

### 4. Tutor State Machine With Agent Slots

Instead of a chat box, use explicit states:

1. Diagnose
2. Ask retrieval prompt
3. Check attempt
4. Choose feedback tier
5. Ask self-explanation
6. Generate repair item
7. Apply review
8. Reflect

Agents fill slots: Socratic prompter, explainer, misconception diagnoser,
rubric grader, safety critic. A deterministic governor validates output and
chooses the next state.

Kernel pressure: attempt events, grade envelope, service command safety.

Client-owned: agent orchestration, prompts, tools, safety policy.

### 5. Study Replay Lab

Every dogfood session becomes a replayable trace:

- source content
- generated prompt
- learner attempt
- confidence
- grade
- feedback
- schedule transition
- queue decision
- model/tool traces
- eval verdicts

This turns "AI tutor quality" into inspectable receipts and creates benchmark
fixtures.

Kernel pressure: event schemas and testkit recipes if repeated.

Client-owned: trace storage, UI, model traces, privacy policy.

## Near-Term Experiments

### AI-Assisted Import Probe

Extend ticket 22's import probe with an optional model-backed compiler over one
tiny fixture. It must output:

- normalized prompts
- progression edges
- duplicate candidates
- source citations
- rejected fields
- validation errors

Do not add provider SDKs to `src`. Use fixtures and static canned model outputs
first; real model calls can come after the schema is stable.

### Rubric Duel

Run the same answers through:

- deterministic grader
- static rubric adapter
- model-backed rubric adapter
- evidence-grounded rubric adapter

Compare verdicts, ratings, feedback, criterion evidence, confidence, and
unsupported claims.

This is the fastest path to know whether current `GradeResult` is thick enough.

### Misconception Repair Lab

Use a small wrong-answer corpus. Retrieve similar mistakes, map them to a
misconception label, generate one repair prompt, and retest.

Success criteria:

- diagnosis precision against gold labels
- no answer leakage in hints
- repair prompt improves second attempt in fixture simulation

### Queue Similarity Lab

Add embeddings or static similarity fixtures to a queue experiment. Compare:

- raw due-first
- current concept/source/domain anti-clump
- semantic contrastive interleaving

Measure selected candidate, diversity, confusable-pair coverage, and latency.

### Socratic Recitation Studio

For passages/proofs/prayers, generate staged prompts:

- worked example
- gist prompt
- cloze
- first-letter cue
- free recitation
- delayed recall

An agent can suggest hints, but progression controls when hints fade.

### Tutor Trace Eval

Build an eval where agents are graded on tool use and pedagogy:

- did it retrieve source evidence?
- did it avoid giving away the answer too early?
- did it ask for self-explanation after a miss?
- did it produce schema-valid commands?
- did it update schedule only after a real attempt?

## Candidate Powder Card Additions

### AI Content Compiler Experiment

Depends on `22-content-normalization-probe`.

Oracle:

- canned model output validates into canonical prompts/progression/queue inputs
- duplicate detection fixture catches near-duplicates
- docs record rejected product-owned fields
- no provider SDK in `src`

### Embedding Similarity Eval

Depends on `20-evals-and-benchmarks-baseline`.

Oracle:

- in-domain similarity fixture with gold duplicate/confusable labels
- local/static embedding adapter test double
- metrics: precision/recall, Recall@k, false merge rate
- docs record threshold and model-version assumptions

### Tutor Trace Schema

Depends on `19-service-boundary-failure-semantics`.

Oracle:

- schema for tutoring state transitions outside the pure kernel
- replay fixture proves attempt -> feedback -> repair -> schedule
- invalid model command is rejected before service execution

### Misconception Repair Dogfood

Depends on AI content compiler and eval baseline.

Oracle:

- wrong-answer corpus maps to misconception labels
- repair prompt generated with evidence and confidence
- second-attempt fixture improves or records no-op

### Evidence-Grounded Rubric Adapter

Depends on modular API and eval baseline.

Oracle:

- adapter accepts retrieved evidence snippets
- rubric result records evidence IDs
- unsupported criterion evidence fails or downgrades
- static/canned tests pass without model calls

## Evaluation Guardrails

- Separate retrieval quality from generated feedback quality.
- Use golden sets with adversarial near-neighbor distractors.
- Randomize answer order in model-judge comparisons.
- Use deterministic graders wherever possible.
- Use multiple judge prompts/models only as second opinion, not truth.
- Track unsupported claims, answer leakage, over-hinting, and learner
  overreliance.
- Re-baseline after every model or prompt change.
- Measure whole learning loops: latency, cost, retention proxy, transfer proxy,
  confidence calibration, and repair success.

## Hard Boundaries

Do not put these in core:

- provider SDKs
- vector stores
- prompt templates
- agent loops
- tutor personas
- content parsers
- model-specific schemas
- learner PII policy
- safety/legal/compliance policy

Do shape stable contracts only after repeated pressure:

- embedding provider adapter
- content normalizer adapter
- evidence-grounded grader adapter
- queue explanation trace
- attempt metadata for confidence/reveal/feedback
- replay/session recipe fixtures
