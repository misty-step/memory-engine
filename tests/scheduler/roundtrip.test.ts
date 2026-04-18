import { describe, expect, test } from 'bun:test';
import { type Card, createEmptyCard, fsrs } from 'ts-fsrs';

import { Rating, type ScheduleState, next } from '../../src';

const controlScheduler = fsrs({
  request_retention: 0.9,
  maximum_interval: 36500,
});

function toScheduleState(card: Card): ScheduleState {
  const { learning_steps: _learningSteps, ...rest } = card;

  return {
    ...rest,
    due: card.due.getTime(),
    last_review: card.last_review?.getTime() ?? null,
  };
}

describe('next', () => {
  test('matches ts-fsrs across a JSON round-trip', () => {
    const t0 = 0;
    const t1 = 24 * 60 * 60 * 1000;

    const first = next(null, Rating.Good, t0);
    const parsed = JSON.parse(JSON.stringify(first)) as ScheduleState;
    const actual = next(parsed, Rating.Good, t1);

    const controlFirst = controlScheduler.next(
      createEmptyCard(new Date(t0)),
      new Date(t0),
      Rating.Good,
    ).card;
    const controlSecond = controlScheduler.next(
      { ...controlFirst, learning_steps: 0 },
      new Date(t1),
      Rating.Good,
    ).card;
    const expected = toScheduleState(controlSecond);

    expect(JSON.stringify(actual)).toBe(JSON.stringify(expected));
  });
});
