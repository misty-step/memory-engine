---
shaping: stub
slice: 3
parent: SPEC.md
status: awaiting-slice-2-completion
---

# Context Packet Stub: Memory Engine — Slice 3 (Rubric + AI-assisted grading)

**This is a strategic stub, not a shaped packet.** Full shaping happens
after slice 2 merges.

## Goal

Add rubric-based and AI-assisted grading paths, expanding the `Prompt`
taxonomy to include `rubric` (and possibly `recitation`), behind a
bounded adapter contract so the kernel stays framework-free and
vendor-free.

## Why this slice exists

Deterministic grading covers the simple majority of prompt forms. But
Vault SRS already runs rubric-based grading with an LLM grader, and
Ruminatio does recitation. These share a common shape — some external
evaluator returns a `GradeResult` envelope that the kernel trusts. Slice
3 codifies that contract so apps stop inventing their own prompt
layers.

## Non-Goals

- Concrete LLM implementation inside the kernel. Kernel ships the
  adapter contract + a no-op reference. Consumers plug in Anthropic,
  OpenAI, or their own prompt pipeline.
- Rubric authoring UI or schema validator beyond the minimum type
  guards needed for runtime safety.
- Prompt engineering details (system prompts, examples, guardrails) —
  stay in the consumer.
- Cost caps, retry policies, rate limiting — consumer responsibility.
- Streaming support in v1. Adapter returns a resolved `GradeResult`.

## Scope

### Rubric types

- `Rubric` — criteria list, passing score, answer guide (lift Vault's
  `RubricDefinition` shape).
- `CriterionResult` — per-criterion verdict + evidence.
- Expansion of `GradeResult` to include `criterionResults?: CriterionResult[]`
  and `graderModel?: string`, `graderConfidence?: number` (optional
  fields; deterministic path leaves them undefined).

### Prompt expansion

- `Prompt` gains a `rubric` arm: `{ kind: 'rubric'; prompt: string;
  rubric: Rubric }`.
- Possibly `recitation` arm if Ruminatio's recitation flow fits (open
  question — it may be an exact-match variant and belong to slice 1
  retroactively).

### Adapter contract

- `GradingAdapter` — single method `gradeRubric(args) →
  Promise<RubricAssessment>` where `RubricAssessment` is the external
  grader's output envelope.
- Confidence threshold wiring: kernel accepts the adapter's confidence
  and maps to `Verdict` via a simple rule (e.g., all required criteria
  pass AND confidence ≥ threshold → correct; some criteria pass →
  close; otherwise wrong). Threshold injectable, not hardcoded.
- Adapter lives in `packages/adapters/grading` (from slice 2).

## Constraints / Invariants

- The adapter contract MUST keep `packages/core` pure — no LLM SDK
  imports in core. Adapter imports/implementations live in `adapters`.
- Rubric path still returns the same `GradeResult` envelope —
  `graderKind` field discriminates (`'rubric-llm'` or similar).
- Adapter receives only `(Prompt, submitted, rubric, ctx)`. Consumer
  wraps with auth, tracing, cost caps, retries.
- Deterministic path remains unaffected. Slice-3 additions are
  additive to `GradeResult` and `Prompt`.

## Open Questions (resolved during slice-3 shaping)

- Is recitation rubric-like (multi-criterion) or exact-match-with-tolerance
  (slice-1-retroactive)? Answer determines whether `recitation` ships
  here or earlier.
- Confidence threshold: codify in core (Vault uses 0.85) or expose as
  config? Leaning toward config with 0.85 default.
- Caching of rubric grading results by `(prompt fingerprint, submitted
  fingerprint)` — core responsibility or adapter responsibility?
  Probably adapter, but the fingerprinting shape may want to live in
  `contracts`.
- Error-path semantics: when adapter fails, does the kernel return
  `verdict: 'revealed'` or throw? Vault current behavior suggests
  throw; prefer throw.

## Depends on

Slice 2 merged (the rubric adapter lives in the slice-2 `adapters`
package).
