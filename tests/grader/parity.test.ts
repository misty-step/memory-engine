import { describe, expect, test } from 'bun:test';

import { gradingFixtures } from 'memory-engine/testkit';
import { Grader } from '../../src';

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
