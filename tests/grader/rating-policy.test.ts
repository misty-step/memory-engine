import { describe, expect, test } from 'bun:test';

import { type GradeCtx, Rating, type Verdict, defaultRatingPolicy } from '../../src';

type RatingCase = {
  verdict: Verdict;
  ctx: GradeCtx;
  expected: number;
};

const timingCases = [
  { label: 'fast', responseTimeMs: 3_000 },
  { label: 'slow', responseTimeMs: 10_000 },
] as const;

const repCases = [0, 3, 10] as const;

const ratingCases: RatingCase[] = [];

for (const verdict of ['correct', 'close', 'wrong', 'revealed'] as const) {
  for (const timingCase of timingCases) {
    for (const priorReps of repCases) {
      const ctx = { responseTimeMs: timingCase.responseTimeMs, priorReps };
      const expected =
        verdict === 'correct'
          ? timingCase.label === 'fast' && priorReps >= 3
            ? Rating.Easy
            : Rating.Good
          : verdict === 'close'
            ? Rating.Hard
            : Rating.Again;

      ratingCases.push({ verdict, ctx, expected });
    }
  }
}

describe('defaultRatingPolicy', () => {
  for (const testCase of ratingCases) {
    const { verdict, ctx, expected } = testCase;
    const label = `${verdict} -> ${expected} for ${ctx.responseTimeMs}ms / reps ${ctx.priorReps}`;

    test(label, () => {
      expect(defaultRatingPolicy(verdict, ctx)).toBe(expected);
    });
  }
});
