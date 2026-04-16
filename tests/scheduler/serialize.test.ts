import { describe, expect, test } from 'bun:test';
import { type Card, createEmptyCard, fsrs } from 'ts-fsrs';

import { Rating, type ScheduleState, next } from '../../src';

type SchedulerFixture = {
  name: string;
  state: ScheduleState;
  now: number;
  rating: Rating;
};

const scheduler = fsrs({
  request_retention: 0.9,
  maximum_interval: 36500,
});

function toScheduleState(card: Card): ScheduleState {
  return {
    ...card,
    due: card.due.getTime(),
    last_review: card.last_review?.getTime() ?? null,
  };
}

function buildFixtures(): SchedulerFixture[] {
  const newState = toScheduleState(createEmptyCard(new Date(0)));
  const learning = toScheduleState(
    scheduler.repeat(createEmptyCard(new Date(0)), new Date(0))[Rating.Good].card,
  );
  const review = toScheduleState(
    scheduler.repeat(
      scheduler.repeat(createEmptyCard(new Date(0)), new Date(0))[Rating.Good].card,
      new Date(learning.due),
    )[Rating.Good].card,
  );
  const relearning = toScheduleState(
    scheduler.repeat(
      scheduler.repeat(
        scheduler.repeat(createEmptyCard(new Date(0)), new Date(0))[Rating.Good].card,
        new Date(learning.due),
      )[Rating.Good].card,
      new Date(review.due),
    )[Rating.Again].card,
  );

  return [
    {
      name: 'new',
      state: newState,
      now: 0,
      rating: Rating.Good,
    },
    {
      name: 'learning',
      state: learning,
      now: learning.due,
      rating: Rating.Good,
    },
    {
      name: 'review',
      state: review,
      now: review.due,
      rating: Rating.Again,
    },
    {
      name: 'relearning',
      state: relearning,
      now: relearning.due,
      rating: Rating.Good,
    },
  ];
}

describe('ScheduleState serialization', () => {
  test.each(buildFixtures())(
    'round-trips losslessly for the $name state and stays schedulable',
    ({ state, now, rating }) => {
      const parsed = JSON.parse(JSON.stringify(state)) as ScheduleState;

      expect(parsed).toEqual(state);
      expect(next(parsed, rating, now)).toEqual(next(state, rating, now));
    },
  );
});
