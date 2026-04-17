import { describe, expect, test } from 'bun:test';

import { recitationFixtures } from 'memory-engine/testkit';
import { Grader } from '../../src';

describe('Grader recitation', () => {
  const grader = new Grader();

  for (const fixture of recitationFixtures) {
    test(fixture.name, () => {
      expect(grader.grade(fixture.prompt, fixture.submitted, fixture.ctx)).toEqual(
        fixture.expected,
      );
    });
  }
});
