import { describe, expect, test } from 'bun:test';

import * as root from 'memory-engine';
import * as grading from 'memory-engine/grading';
import * as progression from 'memory-engine/progression';
import * as queue from 'memory-engine/queue';
import * as scheduling from 'memory-engine/scheduling';
import { Rating, type ReviewUnitId } from 'memory-engine/types';

describe('root barrel compatibility', () => {
  test('keeps existing root exports wired to the modular surfaces', () => {
    expect(root.Grader).toBe(grading.Grader);
    expect(root.AsyncGrader).toBe(grading.AsyncGrader);
    expect(root.defaultRatingPolicy).toBe(grading.defaultRatingPolicy);
    expect(root.resolveRubricGrade).toBe(grading.resolveRubricGrade);
    expect(root.DEFAULT_RUBRIC_CONFIDENCE_FLOOR).toBe(grading.DEFAULT_RUBRIC_CONFIDENCE_FLOOR);

    expect(root.next).toBe(scheduling.next);
    expect(root.isMastered).toBe(progression.isMastered);
    expect(root.filterEligibleCandidates).toBe(progression.filterEligibleCandidates);
    expect(root.filterEligibleCandidatesWithFallback).toBe(
      progression.filterEligibleCandidatesWithFallback,
    );
    expect(root.pickNextQueueCandidate).toBe(queue.pickNextQueueCandidate);
    expect(root.reviewableQueueCandidates).toBe(queue.reviewableQueueCandidates);
    expect(root.compareQueuePriority).toBe(queue.compareQueuePriority);
    expect(root.Rating).toBe(Rating);
  });

  test('preserves the README root-barrel usage path', () => {
    const reviewUnitId = 'latin-1' as ReviewUnitId;
    const grade = new root.Grader().grade(
      {
        kind: 'shortAnswer',
        reviewUnitId,
        prompt: 'Translate poena',
        acceptedAnswers: ['punishment'],
        equivalenceGroups: [],
        ignoredTokens: [],
      },
      'Punishment',
      { responseTimeMs: 3_200, priorReps: 3 },
    );

    expect(root.next(null, grade.rating, Date.UTC(2026, 4, 14))).toMatchObject({
      reps: 1,
      last_review: Date.UTC(2026, 4, 14),
    });
  });
});
