import { describe, expect, test } from 'bun:test';

import type { ReviewUnitId, RubricPrompt } from '../../src';

function reviewUnitId(value: string): ReviewUnitId {
  return value as ReviewUnitId;
}

const prompt: RubricPrompt = {
  kind: 'rubric',
  reviewUnitId: reviewUnitId('adapter-01'),
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

describe('rubric adapter surface', () => {
  test('exports a dedicated adapter subpath with a canned test double', async () => {
    const adapters = await import('memory-engine/adapters');
    const grader = new adapters.StaticRubricGrader({
      model: 'gpt-5.4-mini',
      confidence: 0.93,
      feedback: 'Clear answer.',
      criterionResults: [
        {
          name: 'continuation',
          verdict: 'pass',
          evidence: 'Gives the next line.',
        },
      ],
    });

    await expect(grader.grade(prompt, 'Answer')).resolves.toEqual({
      model: 'gpt-5.4-mini',
      confidence: 0.93,
      feedback: 'Clear answer.',
      criterionResults: [
        {
          name: 'continuation',
          verdict: 'pass',
          evidence: 'Gives the next line.',
        },
      ],
    });
  });
});
