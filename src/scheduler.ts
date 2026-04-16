import { type Card, createEmptyCard, fsrs } from 'ts-fsrs';

import type { Rating, ScheduleState } from './types';

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

function toCard(state: ScheduleState): Card {
  const { last_review, ...rest } = state;

  if (last_review === null) {
    return {
      ...rest,
      due: new Date(state.due),
    };
  }

  return {
    ...rest,
    due: new Date(state.due),
    last_review: new Date(last_review),
  };
}

export function next(state: ScheduleState | null, rating: Rating, now: number): ScheduleState {
  const currentCard = state === null ? createEmptyCard(new Date(now)) : toCard(state);

  return toScheduleState(scheduler.repeat(currentCard, new Date(now))[rating].card);
}
