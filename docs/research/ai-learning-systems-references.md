# AI Learning Systems References

This note captures research and engineering inputs for using modern AI,
embeddings, retrieval, agents, and model-based graders around `memory-engine`.
Provider calls, prompts, vector indexes, and agent loops stay outside the pure
kernel unless repeated experiments prove a stable adapter contract.

Refs-backlog: 20
Refs-backlog: 21
Refs-backlog: 22
Refs-backlog: 23
Refs-backlog: 24

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

### LLM Judges Are Useful But Not Ground Truth

Model graders can scale rubric evaluation, but research and practice show
position bias, style bias, self-consistency issues, prompt-template sensitivity,
and calibration drift. They are best treated as auditable second opinions
calibrated against human or deterministic gold sets.

Design consequence: AI evals need golden sets, multiple judges or templates,
disagreement escalation, reference answers, evidence citations, and regression
tracking. Do not use one LLM judge as the only quality gate.

## API And Experiment Implications

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
- judge agreement against deterministic/human gold labels
- queue diversity and confusable-item contrast
- model cost/latency per whole review loop

### Boundary Rules

- No provider SDKs in `src/`.
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
