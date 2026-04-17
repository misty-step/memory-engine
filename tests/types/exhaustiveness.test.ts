import { describe, expect, test } from 'bun:test';

import { type Prompt, type ReviewUnitId, assertNever } from '../../src';

function reviewUnitId(value: string): ReviewUnitId {
  return value as ReviewUnitId;
}

function promptLabel(prompt: Prompt): string {
  switch (prompt.kind) {
    case 'mcq':
      return prompt.correctChoice;
    case 'boolean':
      return prompt.correctAnswer ? 'true' : 'false';
    case 'cloze':
      return prompt.acceptedAnswers.join(' / ');
    case 'shortAnswer':
      return prompt.acceptedAnswers.join(' / ');
    case 'recitation':
      return prompt.acceptedAnswers.join(' / ');
    default:
      return assertNever(prompt);
  }
}

function missingRecitationArm(prompt: Prompt): string {
  switch (prompt.kind) {
    case 'mcq':
    case 'boolean':
    case 'cloze':
    case 'shortAnswer':
      return prompt.kind;
    default:
      // @ts-expect-error Prompt is not exhaustive without the recitation arm.
      return assertNever(prompt);
  }
}

describe('assertNever', () => {
  test('guards exhaustive Prompt switching', () => {
    const prompt: Prompt = {
      kind: 'mcq',
      reviewUnitId: reviewUnitId('unit-1'),
      prompt: 'Pick one',
      choices: ['A', 'B'],
      correctChoice: 'A',
    };

    expect(promptLabel(prompt)).toBe('A');
    expect(missingRecitationArm(prompt)).toBe('mcq');
  });
});
