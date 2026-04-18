import { describe, expect, test } from 'bun:test';

import { AsyncGrader, Grader, Rating, type ReviewUnitId, type RubricPrompt } from '../../src';
import { StaticRubricGrader } from '../../src/adapters';

function reviewUnitId(value: string): ReviewUnitId {
  return value as ReviewUnitId;
}

const deterministicPrompt = {
  kind: 'shortAnswer' as const,
  reviewUnitId: reviewUnitId('short-answer-01'),
  prompt: 'Answer?',
  acceptedAnswers: ['punishment'],
  equivalenceGroups: [],
  ignoredTokens: [],
};

const rubricPrompt: RubricPrompt = {
  kind: 'rubric',
  reviewUnitId: reviewUnitId('rubric-01'),
  prompt: 'Continue the prayer.',
  rubric: {
    answerGuide: ['Continue with the next line.'],
    passingScore: 1,
    criteria: [
      {
        name: 'continuation',
        description: 'Gives the next line.',
        required: true,
      },
    ],
  },
};

describe('async grading surface', () => {
  test('deterministic grader remains synchronous', () => {
    const result = new Grader().grade(deterministicPrompt, 'Punishment', {
      responseTimeMs: 5_100,
      priorReps: 0,
    });

    expect(result.verdict).toBe('correct');
    expect(result.criterionResults).toEqual([]);
    expect(result).not.toBeInstanceOf(Promise);
  });

  test('async grader preserves deterministic behavior through the additive async surface', async () => {
    const syncResult = new Grader().grade(deterministicPrompt, 'Punishment', {
      responseTimeMs: 5_100,
      priorReps: 0,
    });
    const asyncResult = await new AsyncGrader().grade(deterministicPrompt, 'Punishment', {
      responseTimeMs: 5_100,
      priorReps: 0,
    });

    expect(asyncResult).toEqual(syncResult);
  });

  test('async grader rejects rubric prompts when no adapter is configured', async () => {
    const grader = new AsyncGrader();

    await expect(
      grader.grade(rubricPrompt, 'Some answer', {
        responseTimeMs: 6_000,
        priorReps: 0,
      }),
    ).rejects.toThrow('Rubric grading is unavailable');
  });

  test('async grader handles rubric prompts when an adapter is configured', async () => {
    const grader = new AsyncGrader({
      rubricGrader: new StaticRubricGrader({
        model: 'gpt-5.4-mini',
        confidence: 0.91,
        feedback: 'Strong answer.',
        criterionResults: [
          {
            name: 'continuation',
            verdict: 'pass',
            evidence: 'Continued with the correct line.',
          },
        ],
      }),
    });

    const result = await grader.grade(rubricPrompt, 'Strong answer.', {
      responseTimeMs: 6_000,
      priorReps: 0,
    });

    expect(result.verdict).toBe('correct');
    expect(result.graderKind).toBe('rubric-llm');
  });

  test('async grader applies injected rating policy to rubric prompts', async () => {
    const grader = new AsyncGrader({
      ratingPolicy: (verdict) => (verdict === 'correct' ? Rating.Easy : Rating.Again),
      rubricGrader: new StaticRubricGrader({
        model: 'gpt-5.4-mini',
        confidence: 0.91,
        feedback: 'Strong answer.',
        criterionResults: [
          {
            name: 'continuation',
            verdict: 'pass',
            evidence: 'Continued with the correct line.',
          },
        ],
      }),
    });

    const result = await grader.grade(rubricPrompt, 'Strong answer.', {
      responseTimeMs: 6_000,
      priorReps: 4,
    });

    expect(result.verdict).toBe('correct');
    expect(result.rating).toBe(Rating.Easy);
  });
});
