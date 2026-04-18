import { describe, expect, test } from 'bun:test';

import {
  AsyncGrader,
  type ReviewUnitId,
  type RubricCriterionResult,
  type RubricPrompt,
} from '../../src';
import { StaticRubricGrader } from '../../src/adapters';

function reviewUnitId(value: string): ReviewUnitId {
  return value as ReviewUnitId;
}

function rubricPrompt(): RubricPrompt {
  return {
    kind: 'rubric',
    reviewUnitId: reviewUnitId('mass-rubric-01'),
    prompt: 'At Mass, before the Gospel, the deacon announces the reading. Respond.',
    rubric: {
      answerGuide: ['Say "Glory to you, O Lord."'],
      passingScore: 2,
      criteria: [
        {
          name: 'speaker',
          description: 'Understands this is the Gospel response.',
          required: true,
        },
        {
          name: 'response',
          description: 'Supplies the correct response.',
          required: true,
        },
      ],
    },
  };
}

function criterionResults(
  verdicts: Record<string, RubricCriterionResult['verdict']>,
): RubricCriterionResult[] {
  return [
    {
      name: 'speaker',
      verdict: verdicts.speaker ?? 'fail',
      evidence: 'speaker evidence',
    },
    {
      name: 'response',
      verdict: verdicts.response ?? 'fail',
      evidence: 'response evidence',
    },
  ];
}

describe('rubric grading contract', () => {
  test('returns correct when required criteria, passing score, and confidence all clear', async () => {
    const grader = new AsyncGrader({
      rubricGrader: new StaticRubricGrader({
        model: 'gpt-5.4-mini',
        confidence: 0.92,
        feedback: 'Clear and complete.',
        criterionResults: criterionResults({
          speaker: 'pass',
          response: 'pass',
        }),
      }),
    });

    const result = await grader.grade(rubricPrompt(), 'Glory to you, O Lord.', {
      responseTimeMs: 6_000,
      priorReps: 0,
    });

    expect(result.verdict).toBe('correct');
    expect(result.rating).toBe(3);
    expect(result.isCorrect).toBe(true);
    expect(result.graderKind).toBe('rubric-llm');
    expect(result.feedback).toBe('Clear and complete.');
    expect(result.criterionResults).toEqual(
      criterionResults({
        speaker: 'pass',
        response: 'pass',
      }),
    );
  });

  test('downgrades low-confidence rubric passes to close', async () => {
    const grader = new AsyncGrader({
      rubricGrader: new StaticRubricGrader({
        model: 'gpt-5.4-mini',
        confidence: 0.62,
        feedback: 'Mostly right, but the evidence is thin.',
        criterionResults: criterionResults({
          speaker: 'pass',
          response: 'pass',
        }),
      }),
    });

    const result = await grader.grade(rubricPrompt(), 'Something close enough.', {
      responseTimeMs: 6_000,
      priorReps: 0,
    });

    expect(result.verdict).toBe('close');
    expect(result.rating).toBe(2);
    expect(result.isCorrect).toBe(false);
  });

  test('downgrades missing required criteria to close even when passing score is met', async () => {
    const grader = new AsyncGrader({
      rubricGrader: new StaticRubricGrader({
        model: 'gpt-5.4-mini',
        confidence: 0.98,
        feedback: 'Got one criterion but missed the required continuation.',
        criterionResults: [
          {
            name: 'speaker',
            verdict: 'fail',
            evidence: 'Did not establish the response context.',
          },
          {
            name: 'response',
            verdict: 'pass',
            evidence: 'Supplied the line itself.',
          },
        ],
      }),
    });

    const result = await grader.grade(rubricPrompt(), 'Glory to you, O Lord.', {
      responseTimeMs: 6_000,
      priorReps: 0,
    });

    expect(result.verdict).toBe('close');
    expect(result.rating).toBe(2);
    expect(result.isCorrect).toBe(false);
  });
});
