import { describe, expect, test } from 'bun:test';

import { type Prompt, type ReviewUnitId, assertNever } from '../../src';

function reviewUnitId(value: string): ReviewUnitId {
  return value as ReviewUnitId;
}

function gradeablePromptKind(prompt: Prompt): Prompt['kind'] {
  switch (prompt.kind) {
    case 'mcq':
    case 'boolean':
    case 'cloze':
    case 'shortAnswer':
      return prompt.kind;
    default:
      return assertNever(prompt);
  }
}

function missingShortAnswerArm(prompt: Prompt): Prompt['kind'] {
  switch (prompt.kind) {
    case 'mcq':
    case 'boolean':
    case 'cloze':
      return prompt.kind;
    default:
      // @ts-expect-error Prompt is not exhaustive without the shortAnswer arm.
      return assertNever(prompt);
  }
}

describe('grader exhaustiveness', () => {
  test('covers every Prompt arm', () => {
    const prompt: Prompt = {
      kind: 'mcq',
      reviewUnitId: reviewUnitId('unit-1'),
      prompt: 'Pick one',
      choices: ['A', 'B'],
      correctChoice: 'A',
    };

    expect(gradeablePromptKind(prompt)).toBe('mcq');
    expect(missingShortAnswerArm(prompt)).toBe('mcq');
  });
});
