import type { RubricGraderAdapter } from './adapters';
import { Grader, defaultRatingPolicy } from './grader';
import { Rating } from './types';
import type {
  GradeCtx,
  GradeResult,
  Prompt,
  RatingPolicy,
  RubricAssessment,
  RubricCriterionResult,
  RubricPrompt,
  Verdict,
} from './types';

export const DEFAULT_RUBRIC_CONFIDENCE_FLOOR = 0.85;

type AsyncGraderOptions = {
  ratingPolicy?: RatingPolicy;
  rubricGrader?: RubricGraderAdapter | null;
  rubricConfidenceFloor?: number;
};

export class AsyncGrader {
  readonly #deterministicGrader: Grader;
  readonly #ratingPolicy: RatingPolicy;
  readonly #rubricGrader: RubricGraderAdapter | null;
  readonly #rubricConfidenceFloor: number;

  constructor(options: AsyncGraderOptions = {}) {
    this.#ratingPolicy = options.ratingPolicy ?? defaultRatingPolicy;
    this.#deterministicGrader =
      options.ratingPolicy === undefined
        ? new Grader()
        : new Grader({
            ratingPolicy: options.ratingPolicy,
          });
    this.#rubricGrader = options.rubricGrader ?? null;
    this.#rubricConfidenceFloor = options.rubricConfidenceFloor ?? DEFAULT_RUBRIC_CONFIDENCE_FLOOR;
  }

  async grade(
    prompt: Prompt | RubricPrompt,
    submitted: string,
    ctx: GradeCtx,
  ): Promise<GradeResult> {
    if (prompt.kind !== 'rubric') {
      return this.#deterministicGrader.grade(prompt, submitted, ctx);
    }

    return this.gradeRubric(prompt, submitted, ctx);
  }

  async gradeRubric(
    prompt: RubricPrompt,
    submitted: string,
    ctx: GradeCtx = {
      responseTimeMs: 0,
      priorReps: 0,
    },
  ): Promise<GradeResult> {
    const submittedAnswer = submitted.trim();

    if (submittedAnswer.length === 0) {
      return rubricGrade(
        'wrong',
        Rating.Again,
        submittedAnswer,
        rubricExpectedAnswer(prompt),
        false,
        null,
        1,
        'No answer submitted.',
        prompt.rubric.criteria.map((criterion) => ({
          name: criterion.name,
          verdict: 'fail',
          evidence: 'No answer submitted.',
        })),
      );
    }

    const rubricGrader = this.#rubricGrader;
    if (rubricGrader === null) {
      throw new Error('Rubric grading is unavailable');
    }

    const assessment = await rubricGrader.grade(prompt, submittedAnswer);
    return resolveRubricGrade(
      prompt,
      submittedAnswer,
      assessment,
      this.#rubricConfidenceFloor,
      this.#ratingPolicy,
      ctx,
    );
  }
}

export function resolveRubricGrade(
  prompt: RubricPrompt,
  submittedAnswer: string,
  assessment: RubricAssessment,
  confidenceFloor = DEFAULT_RUBRIC_CONFIDENCE_FLOOR,
  ratingPolicy: RatingPolicy = defaultRatingPolicy,
  ctx: GradeCtx = {
    responseTimeMs: 0,
    priorReps: 0,
  },
): GradeResult {
  const criterionResults = normalizeCriterionResults(prompt, assessment);
  const passedCount = criterionResults.filter((criterion) => criterion.verdict === 'pass').length;
  const requiredSatisfied = prompt.rubric.criteria.every((criterion) => {
    if (!criterion.required) {
      return true;
    }

    return criterionResults.some(
      (result) => result.name === criterion.name && result.verdict === 'pass',
    );
  });
  const hasPassingScore = passedCount >= prompt.rubric.passingScore;
  const confidentEnough = assessment.confidence >= confidenceFloor;
  const verdict: Verdict =
    requiredSatisfied && hasPassingScore && confidentEnough
      ? 'correct'
      : passedCount > 0
        ? 'close'
        : 'wrong';
  const rating = ratingPolicy(verdict, ctx);

  return rubricGrade(
    verdict,
    rating,
    submittedAnswer,
    rubricExpectedAnswer(prompt),
    verdict === 'correct',
    assessment.model,
    clampConfidence(assessment.confidence),
    assessment.feedback.trim().length > 0
      ? assessment.feedback.trim()
      : 'Answer did not clearly satisfy the rubric.',
    criterionResults,
  );
}

function normalizeCriterionResults(
  prompt: RubricPrompt,
  assessment: RubricAssessment,
): RubricCriterionResult[] {
  const byName = new Map(
    assessment.criterionResults
      .map((criterion) => [normalizeKey(criterion.name), criterion] as const)
      .filter(([name]) => name.length > 0),
  );

  return prompt.rubric.criteria.map((criterion) => {
    const match = byName.get(normalizeKey(criterion.name));
    return {
      name: criterion.name,
      verdict: match?.verdict === 'pass' ? 'pass' : 'fail',
      evidence:
        typeof match?.evidence === 'string' && match.evidence.trim().length > 0
          ? match.evidence.trim()
          : 'No clear evidence supplied.',
    };
  });
}

function normalizeKey(value: string): string {
  return value.trim().toLowerCase();
}

function rubricExpectedAnswer(prompt: RubricPrompt): string {
  return (
    prompt.rubric.answerGuide.join(' / ') ||
    prompt.rubric.criteria.map((criterion) => criterion.name).join(' / ')
  );
}

function clampConfidence(value: number): number {
  if (!Number.isFinite(value)) {
    return 0;
  }

  return Math.min(1, Math.max(0, value));
}

function rubricGrade(
  verdict: Verdict,
  rating: GradeResult['rating'],
  submittedAnswer: string,
  expectedAnswer: string,
  isCorrect: boolean,
  graderModel: string | null,
  graderConfidence: number | null,
  feedback: string,
  criterionResults: RubricCriterionResult[],
): GradeResult {
  return {
    verdict,
    rating,
    isCorrect,
    submittedAnswer,
    expectedAnswer,
    graderKind: 'rubric-llm',
    graderModel,
    graderConfidence,
    feedback,
    criterionResults,
  };
}
