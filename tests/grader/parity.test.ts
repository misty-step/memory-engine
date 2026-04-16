import { describe, expect, test } from 'bun:test';

import { Grader } from '../../src';
import { gradingFixtures } from './fixtures';

describe('Grader parity', () => {
  const grader = new Grader();

  for (const fixture of gradingFixtures) {
    test(fixture.name, () => {
      expect(grader.grade(fixture.prompt, fixture.submitted, fixture.ctx)).toEqual(
        fixture.expected,
      );
    });
  }
});
