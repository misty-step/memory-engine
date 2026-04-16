import { assertNever } from './assert';
import type {
  BooleanPrompt,
  ClozePrompt,
  GradeCtx,
  GradeResult,
  McqPrompt,
  Prompt,
  Rating,
  RatingPolicy,
  ShortAnswerPrompt,
  Verdict,
} from './types';
import { Rating as FsrsRating } from './types';

export function defaultRatingPolicy(verdict: Verdict, ctx: GradeCtx): Rating {
  if (verdict === 'correct') {
    if (ctx.priorReps >= 3 && ctx.responseTimeMs <= 4_000) {
      return FsrsRating.Easy;
    }

    return FsrsRating.Good;
  }

  if (verdict === 'close') {
    return FsrsRating.Hard;
  }

  return FsrsRating.Again;
}

type GraderOptions = {
  ratingPolicy?: RatingPolicy;
};

export class Grader {
  readonly #ratingPolicy: RatingPolicy;

  constructor(options: GraderOptions = {}) {
    this.#ratingPolicy = options.ratingPolicy ?? defaultRatingPolicy;
  }

  grade(prompt: Prompt, submitted: string, ctx: GradeCtx): GradeResult {
    const submittedAnswer = submitted.trim();

    switch (prompt.kind) {
      case 'mcq':
        return this.gradeMcq(prompt, submittedAnswer, ctx);
      case 'boolean':
        return this.gradeBoolean(prompt, submittedAnswer, ctx);
      case 'cloze':
        return this.gradeExact(prompt, submittedAnswer, ctx);
      case 'shortAnswer':
        return this.gradeExact(prompt, submittedAnswer, ctx);
      default:
        return assertNever(prompt);
    }
  }

  private gradeMcq(prompt: McqPrompt, submittedAnswer: string, ctx: GradeCtx): GradeResult {
    const isCorrect = submittedAnswer === prompt.correctChoice;
    const verdict: Verdict = isCorrect ? 'correct' : 'wrong';

    return deterministicGrade(
      verdict,
      this.#ratingPolicy(verdict, ctx),
      submittedAnswer,
      prompt.correctChoice,
      isCorrect,
    );
  }

  private gradeBoolean(prompt: BooleanPrompt, submittedAnswer: string, ctx: GradeCtx): GradeResult {
    const expectedAnswer = prompt.correctAnswer ? 'True' : 'False';
    const isCorrect = normalizeExact(submittedAnswer) === normalizeExact(expectedAnswer);
    const verdict: Verdict = isCorrect ? 'correct' : 'wrong';

    return deterministicGrade(
      verdict,
      this.#ratingPolicy(verdict, ctx),
      submittedAnswer,
      expectedAnswer,
      isCorrect,
    );
  }

  private gradeExact(
    prompt: ClozePrompt | ShortAnswerPrompt,
    submittedAnswer: string,
    ctx: GradeCtx,
  ): GradeResult {
    const normalizedSubmitted = normalizeForComparison(
      submittedAnswer,
      prompt.equivalenceGroups,
      prompt.ignoredTokens,
    );
    const normalizedAccepted = prompt.acceptedAnswers.map((answer) =>
      normalizeForComparison(answer, prompt.equivalenceGroups, prompt.ignoredTokens),
    );

    if (normalizedAccepted.includes(normalizedSubmitted)) {
      return deterministicGrade(
        'correct',
        this.#ratingPolicy('correct', ctx),
        submittedAnswer,
        expectedAnswer(prompt),
        true,
      );
    }

    const isClose =
      normalizedSubmitted.length > 0 &&
      normalizedAccepted.some((candidate) => {
        if (!candidate) {
          return false;
        }

        return levenshtein(normalizedSubmitted, candidate) <= nearMissThreshold(candidate.length);
      });
    const verdict: Verdict = isClose ? 'close' : 'wrong';

    return deterministicGrade(
      verdict,
      this.#ratingPolicy(verdict, ctx),
      submittedAnswer,
      expectedAnswer(prompt),
      false,
    );
  }
}

function expectedAnswer(prompt: ClozePrompt | ShortAnswerPrompt): string {
  return prompt.acceptedAnswers.join(' / ');
}

function normalizeExact(value: string): string {
  return normalizeForComparison(value, [], []);
}

function normalizeForComparison(
  value: string,
  equivalenceGroups: string[][],
  ignoredTokens: string[],
): string {
  let normalized = baseNormalize(value);
  if (!normalized) {
    return normalized;
  }

  normalized = applyEquivalenceGroups(normalized, equivalenceGroups);
  normalized = stripIgnoredTokens(normalized, ignoredTokens);

  return normalized.replace(/\s+/g, ' ').trim();
}

function baseNormalize(value: string): string {
  return value
    .normalize('NFKD')
    .replace(/\p{Diacritic}/gu, '')
    .replace(/[^\p{Letter}\p{Number}\s]/gu, ' ')
    .replace(/\s+/g, ' ')
    .trim()
    .toLowerCase();
}

function applyEquivalenceGroups(value: string, groups: string[][]): string {
  if (groups.length === 0) {
    return value;
  }

  const replacements = groups
    .map((group) => group.map(baseNormalize).filter((entry) => entry.length > 0))
    .filter((group) => group.length >= 2)
    .flatMap((group) => {
      const [canonical, ...aliases] = Array.from(new Set(group));
      return aliases.map((alias) => [alias, canonical] as const);
    })
    .sort((left, right) => right[0].length - left[0].length);

  return replacements.reduce((current, [alias, canonical]) => {
    const pattern = new RegExp(`(^| )${escapeRegExp(alias)}(?= |$)`, 'g');
    return current.replace(pattern, (_match, prefix: string) => `${prefix}${canonical}`);
  }, value);
}

function stripIgnoredTokens(value: string, ignoredTokens: string[]): string {
  if (ignoredTokens.length === 0) {
    return value;
  }

  const patterns = ignoredTokens
    .map(baseNormalize)
    .filter((entry) => entry.length > 0)
    .sort((left, right) => right.length - left.length);

  return patterns.reduce((current, ignored) => {
    const pattern = new RegExp(`(^| )${escapeRegExp(ignored)}(?= |$)`, 'g');
    return current.replace(pattern, (_match, prefix: string) => prefix);
  }, value);
}

function nearMissThreshold(length: number): number {
  if (length <= 4) {
    return 0;
  }
  if (length <= 8) {
    return 1;
  }
  return 2;
}

function levenshtein(left: string, right: string): number {
  if (left === right) {
    return 0;
  }
  if (left.length === 0) {
    return right.length;
  }
  if (right.length === 0) {
    return left.length;
  }

  let previous = Array.from({ length: right.length + 1 }, (_, index) => index);

  for (let row = 1; row <= left.length; row += 1) {
    const current = [row];

    for (let column = 1; column <= right.length; column += 1) {
      const cost = left[row - 1] === right[column - 1] ? 0 : 1;

      current[column] = Math.min(
        (previous[column] ?? Number.POSITIVE_INFINITY) + 1,
        (current[column - 1] ?? Number.POSITIVE_INFINITY) + 1,
        (previous[column - 1] ?? Number.POSITIVE_INFINITY) + cost,
      );
    }

    previous = current;
  }

  return previous[right.length] ?? 0;
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function deterministicGrade(
  verdict: Verdict,
  rating: Rating,
  submittedAnswer: string,
  expectedAnswer: string,
  isCorrect: boolean,
): GradeResult {
  return {
    verdict,
    rating,
    isCorrect,
    submittedAnswer,
    expectedAnswer,
    graderKind: 'deterministic',
    graderModel: null,
    graderConfidence: null,
    feedback: '',
  };
}
