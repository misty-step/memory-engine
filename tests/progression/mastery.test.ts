import { describe, expect, test } from 'bun:test';
import { State } from 'ts-fsrs';

import { isMastered } from '../../src';
import type { MasteryPolicy } from '../../src';

type Review = {
  reps: number;
  state: number;
};

const ruminatioMastery: MasteryPolicy<Review> = (review) => {
  return review.state >= State.Review || review.reps >= 2;
};

const vaultMastery: MasteryPolicy<Review> = (review) => {
  return review.state === State.Review && review.reps >= 3;
};

describe('isMastered', () => {
  test('treats null review as not mastered for every policy', () => {
    expect(isMastered(null, ruminatioMastery)).toBeFalse();
    expect(isMastered(null, vaultMastery)).toBeFalse();
  });

  test('accepts Ruminatio-style mastery thresholds', () => {
    const review = { state: State.Review, reps: 2 };

    expect(isMastered(review, ruminatioMastery)).toBeTrue();
    expect(isMastered(review, vaultMastery)).toBeFalse();
  });

  test('accepts Vault-style mastery thresholds', () => {
    const review = { state: State.Review, reps: 4 };

    expect(isMastered(review, ruminatioMastery)).toBeTrue();
    expect(isMastered(review, vaultMastery)).toBeTrue();
  });
});
