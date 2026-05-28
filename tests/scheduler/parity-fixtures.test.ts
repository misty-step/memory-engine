import { describe, expect, test } from 'bun:test';

import schedulerFixtures from '../../fixtures/scheduler.json';
import { type Rating, type ScheduleState, next } from '../../src';

type SchedulerJsonFixture = {
  name: string;
  initialState: ScheduleState;
  rating: Rating;
  now: number;
  expected: ScheduleState;
};

describe('scheduler shared parity fixtures', () => {
  test.each(schedulerFixtures as SchedulerJsonFixture[])('$name', (fixture) => {
    expect(next(fixture.initialState, fixture.rating, fixture.now)).toEqual(fixture.expected);
  });
});
