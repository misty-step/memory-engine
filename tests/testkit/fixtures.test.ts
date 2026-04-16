import { describe, expect, test } from 'bun:test';

import { gradingFixtures, schedulerFixtures } from 'memory-engine/testkit';
import { Grader, next } from '../../src';

describe('testkit fixtures', () => {
  test('exports non-empty grading fixtures', () => {
    expect(gradingFixtures.length).toBeGreaterThan(0);
  });

  test('exports non-empty scheduler fixtures', () => {
    expect(schedulerFixtures.length).toBeGreaterThan(0);
  });

  test('grading fixtures stay in sync with the live grader', () => {
    const grader = new Grader();

    for (const fixture of gradingFixtures) {
      expect(grader.grade(fixture.prompt, fixture.submitted, fixture.ctx)).toEqual(
        fixture.expected,
      );
    }
  });

  test('scheduler fixtures stay in sync with the live scheduler', () => {
    for (const fixture of schedulerFixtures) {
      expect(next(fixture.initialState, fixture.rating, fixture.now)).toEqual(fixture.expected);
    }
  });
});
