# AI Learning Systems References

This note captures research and engineering inputs for using modern AI,
embeddings, retrieval, agents, and model-based graders around `memory-engine`.
Provider calls, prompts, vector indexes, and agent loops stay outside the pure
kernel unless repeated experiments prove a stable adapter contract.


## Core Findings

### Embeddings Are Useful For Similarity, Not Truth

Embedding models support semantic search, clustering, recommendations,
classification, anomaly detection, duplicate detection, and diversity analysis.
They are especially useful for content normalization and queue enrichment, but
they do not prove correctness.

Design consequence: use embeddings to propose candidates, clusters, aliases,
confusable items, and retrieval contexts. Keep correctness decisions in
deterministic graders, rubric contracts, or explicitly-evaluated model graders.

### RAG Needs Separate Retrieval And Generation Evals

Retrieval-augmented generation can ground tutoring, grading, feedback, and
content generation in a known corpus. But retrieval quality and generated-answer
faithfulness can fail independently. A good answer may mask poor retrieval, and
good retrieval may still lead to unsupported generation.

Design consequence: eval retrieval with Recall@k, MRR, nDCG, context precision,
and near-neighbor distractors. Eval generation with evidence support,
unsupported-claim checks, rubric agreement, and adversarial context omission.

### Agents Help When Workflow State Is Explicit

Tool-using agents are now a concrete pattern: the model selects a tool, the app
executes it, then the model consumes tool results in a loop. ReAct, Reflexion,
and Self-Refine support planner/evaluator and feedback-driven refinement
patterns, but quality depends on the feedback and guardrails.

Design consequence: client agents should run inside deterministic state
machines such as Diagnose -> Socratic probe -> Attempt -> Feedback -> Repair ->
Schedule -> Reflect. Agent output should be structured and validated.

### Tutoring Needs Pedagogy, Not Just Chat

Intelligent tutoring systems and newer LLM tutoring studies suggest the
promising path is not free-form answer chat. Better systems use scaffolding,
hint policy, worked examples, stepwise feedback, Socratic prompts, and explicit
guardrails. Human-AI tutor-support systems can improve tutoring quality when
they surface targeted, pedagogically useful suggestions.

Design consequence: dogfood clients should be narrow tutoring workflows, not a
generic chat box. The model should propose hints, repairs, examples, or
diagnoses that are checked against learning-state contracts.

### Generated Exercises Need Validation, Not Just Variety

Generative practice is most valuable when it creates fresh problems that train
the same underlying concept under varied conditions. It is also risky: a model
can produce ambiguous questions, wrong worked solutions, answer leakage, or
problems that do not actually test the intended concept.

Design consequence: exercise generation must persist activity kind, concept
target, difficulty/stage, source or rule provenance, expected solution, grading
rubric, validation status, and critique notes. Keep generation and validation
in the beta layer until repeated dogfood evidence proves a stable contract.

### LLM Judges Are Useful But Not Ground Truth

Model graders can scale rubric evaluation, but research and practice show
position bias, style bias, self-consistency issues, prompt-template sensitivity,
and calibration drift. They are best treated as auditable second opinions
calibrated against human or deterministic gold sets.

Design consequence: AI evals need golden sets, multiple judges or templates,
disagreement escalation, reference answers, evidence citations, and regression
tracking. Do not use one LLM judge as the only quality gate.

## API And Experiment Implications

## Evidence Matrix

This matrix records which AI-system findings should shape the beta interface
and which should stay out of the pure kernel until proven by dogfood evidence.

| Evidence | What it supports | Product/API consequence |
| --- | --- | --- |
| OpenAI embeddings guidance, MTEB, BEIR, and Sentence-Transformers examples frame embeddings as retrieval, clustering, classification, and similarity tools. | Semantic similarity can help de-duplicate, cluster, recommend, and find reference context, but it is not truth. | Beta can maintain a client-owned embedding/index layer for content organization; correctness still comes from deterministic grading, rubric contracts, or evaluated model graders. |
| TREC RAG and retrieval benchmark practice separate retrieval quality from answer generation quality. | RAG needs separate retrieval and generation evals. | Content generation should store source passages, retrieval receipts, model/version metadata, and unsupported-claim checks before generated quizzes enter the review queue. |
| ReAct, Reflexion, and Self-Refine show agent loops can improve task execution when observations and feedback are explicit. | Agents need explicit workflow state, not free-form hidden autonomy. | Beta generation should use structured states such as ingest -> normalize -> retrieve -> draft prompts -> critique -> approve/save, with validated outputs at each step. |
| VanLehn's tutoring-systems review and newer AutoTutor/LLM tutoring work suggest tutoring quality comes from scaffolded pedagogy, not generic chat. | A learning agent should produce hints, worked examples, Socratic probes, exercises, and repair material under policy. | The interface should offer narrow tutor actions from a review item; the kernel should not embed tutor prompts. |
| Cognitive-load and interleaving research suggest worked examples, faded guidance, and varied problem solving support transfer. | Generated exercises should be staged and validated, not sprayed randomly. | Beta generation should start with deterministic exercise templates and only then add model-generated scenario variants with stored worked solutions and evals. |
| Tutor CoPilot and Socratic tutoring work point toward AI assisting high-context pedagogical decisions. | AI can help select next intervention when user state and content evidence are available. | Store attempt history, struggle patterns, concept/source metadata, and reference links so future agents can diagnose failure loops. |
| LLM-judge reliability literature and production grader guidance show model judgments have bias and drift. | Model grading is useful but cannot be the only gate. | Rubric/model graders need golden examples, disagreement handling, confidence, evidence citations, and regression tracking. |
| NIST AI TEVV and GenAI evaluation guidance emphasize test, evaluation, validation, and verification as ongoing governance. | AI behavior must be measured over time. | Add evals for duplicate detection, retrieval Recall@k, hint leakage, unsupported claims, judge agreement, latency, and cost before promoting AI contracts. |
| Prompt-injection and data-control guidance make source permissions part of product design. | User-owned content and web/file ingestion require explicit safety boundaries. | Beta persistence must track source provenance, permission labels, model-send eligibility, and generated-content provenance. |

## Beta AI Workflow Requirements

The beta interface may need a database immediately. That does not imply the
published pure kernel should own one. The product shell should persist enough
state to make AI-assisted learning usable and auditable:

- `SourceDocument`: user-entered text, uploaded file metadata, link metadata,
  image/video transcript references, source permissions, and freshness.
- `ReferenceSpan`: cited excerpts or ranges used to generate or explain a
  prompt.
- `GeneratedPromptDraft`: model/provider/version, input source ids, prompt
  text, accepted answers, rubric criteria, confidence, activity kind, ladder
  stage, and critique status.
- `GeneratedExerciseDraft`: concept target, difficulty/stage, prompt, worked
  solution, scoring rubric, source/rule provenance, validation status, and
  critique notes.
- `ReviewUnitRecord`: canonical prompt plus learner-owned schedule state,
  concept/source/domain keys, references, and supersession/progression metadata.
- `AttemptRecord`: answer, latency, confidence, reveal state, verdict, rating,
  feedback, schedule update, and repair hints shown.
- `GenerationRun`: structured agent run receipts, tool inputs, validation
  failures, cost/latency, and final saved artifacts.

Keep these records in the beta application layer until repeated clients need
the same contract. Promote only stable, provider-neutral contracts back into
`memory-engine`.

## Ticket Mapping

- `26-beta-persistence-spine`: stores source, draft, attempt, schedule,
  reference, and generation-run records so AI output remains auditable.
- `27-ai-content-generation-probe`: proves source-grounded prompt generation,
  exercise generation, critique, rejection, approval, and provenance before
  model output reaches the review queue.
- `28-mobile-beta-study-interface`: tests whether AI-generated content improves
  an actual review/practice loop instead of producing disconnected artifacts.
- `29-service-contract-v0-hardening`: decides which DTOs, reveal semantics, and
  error/idempotency contracts are stable enough to survive durable AI-assisted
  workflows.
- `31-beta-extraction-decision`: uses beta evidence to decide whether any AI
  adapters or helpers deserve promotion.

### Candidate Adapter Contracts

Only shape these after dogfood pressure:

- `EmbeddingProvider`: turns text into vectors with model/version metadata.
- `SimilarityIndex`: nearest-neighbor search over client-owned vectors.
- `ContentNormalizer`: maps authored input into canonical prompts/candidates.
- `MisconceptionDiagnoser`: maps wrong answers to likely misconception IDs.
- `HintGenerator`: proposes hints with provenance and confidence.
- `TutorTurnPlanner`: proposes the next pedagogical action under a state
  machine.
- `EvidenceGrader`: grades answers against retrieved evidence and rubric
  criteria.

### Candidate AI Evals

- duplicate detection precision/recall
- retrieval Recall@k for canonical source passages
- misconception diagnosis precision
- hint helpfulness and answer leakage rate
- unsupported-claim rate in model feedback
- generated exercise solvability and solution correctness
- activity-stage calibration: whether generated problems match intended
  difficulty
- judge agreement against deterministic/human gold labels
- queue diversity and confusable-item contrast
- model cost/latency per whole review loop

### Boundary Rules

- No provider SDKs in `crates/memory-engine-core`.
- No vector store or prompt template in the kernel.
- No model output accepted without schema validation and behavior evals.
- No AI-generated content promoted to testkit without provenance.
- No tutoring agent loop promoted to API after one client.
- No learner data sent to hosted models without explicit product-level policy.

## Source Index

### Official AI Docs

- OpenAI embeddings guide:
  https://platform.openai.com/docs/guides/embeddings
- OpenAI embedding models:
  https://platform.openai.com/docs/guides/embeddings/embedding-models
- OpenAI file search:
  https://platform.openai.com/docs/guides/tools-file-search
- OpenAI function calling:
  https://platform.openai.com/docs/guides/function-calling
- OpenAI Structured Outputs:
  https://platform.openai.com/docs/guides/structured-outputs
- OpenAI Agents SDK:
  https://platform.openai.com/docs/guides/agents-sdk/
- OpenAI Agents SDK tools:
  https://openai.github.io/openai-agents-js/guides/tools/
- OpenAI Agents SDK guardrails:
  https://openai.github.io/openai-agents-js/guides/guardrails/
- OpenAI graders:
  https://platform.openai.com/docs/guides/graders/
- OpenAI Evals API:
  https://platform.openai.com/docs/api-reference/evals/create

### Retrieval And Embeddings Benchmarks

- MTEB:
  https://arxiv.org/abs/2210.07316
- BEIR:
  https://arxiv.org/abs/2104.08663
- TREC RAG:
  https://pages.nist.gov/trec-browser/trec33/rag/overview/
- pgvector:
  https://github.com/pgvector/pgvector
- FAISS:
  https://github.com/facebookresearch/faiss
- Sentence-Transformers semantic search:
  https://www.sbert.net/examples/sentence_transformer/applications/semantic-search/README.html

### Agents And Tutoring

- ReAct:
  https://arxiv.org/abs/2210.03629
- Reflexion:
  https://arxiv.org/abs/2303.11366
- Self-Refine:
  https://proceedings.neurips.cc/paper_files/paper/2023/file/91edff07232fb1b55a505a9e9f6c0ff3-Paper-Conference.pdf
- VanLehn, tutoring systems review:
  https://www.tandfonline.com/doi/abs/10.1080/00461520.2011.611369
- AutoTutor with LLMs:
  https://arxiv.org/abs/2402.09216
- Tutor CoPilot:
  https://arxiv.org/abs/2410.03017
- Socratic Playground for Learning:
  https://arxiv.org/abs/2406.13919
- Pedagogical Steering / LLM tutoring work:
  https://aclanthology.org/2025.findings-acl.1348.pdf

### Evaluation And Reliability

- NIST AI TEVV:
  https://www.nist.gov/ai-test-evaluation-validation-and-verification-tevv
- NIST GenAI evaluation:
  https://www.nist.gov/programs-projects/generative-artificial-intelligence-evaluation-program-genai
- TruthfulQA:
  https://arxiv.org/abs/2109.07958
- SelfCheckGPT:
  https://arxiv.org/abs/2303.08896
- FActScore:
  https://arxiv.org/abs/2305.14251
- LLM judge bias/consistency tutorial:
  https://www.frontiersin.org/journals/education/articles/10.3389/feduc.2023.1272229/full
- ETS e-rater:
  https://www.ets.org/erater/about.html
- ETS e-rater quality controls:
  https://www.ets.org/content/dam/ets-org/Media/Research/pdf/RD_Connections2.pdf

### Safety And Privacy

- OpenAI prompt-injection safety:
  https://openai.com/safety/prompt-injections/
- OpenAI data controls:
  https://developers.openai.com/api/docs/guides/your-data
- OpenAI agentic AI governance practices:
  https://cdn.openai.com/papers/practices-for-governing-agentic-ai-systems.pdf
- COPPA:
  https://www.ftc.gov/legal-library/browse/rules/childrens-online-privacy-protection-rule-coppa
- FERPA:
  https://www.law.cornell.edu/cfr/text/34/part-99/subpart-A
