import { describe, expect, test } from 'bun:test';

import { runCliReview } from './index';

describe('cli review dogfood client', () => {
  test('runs a calibration-aware review loop through the service boundary', async () => {
    const receipt = await runCliReview();

    expect(receipt).toMatchObject({
      fixture: 'latin-prayer-opening',
      commands: ['grade/apply-review', 'next-queue'],
      confidence: 0.72,
      calibrationError: 0.28,
      attemptCount: 1,
      gradedVerdict: 'correct',
      scheduledReps: 1,
      nextReviewUnitId: 'cli-pater-opening',
    });
    expect(receipt.gradedRating).toBeGreaterThan(0);
    expect(receipt.stayedOutsideSrc).toContain('confidence capture');
  });
});
