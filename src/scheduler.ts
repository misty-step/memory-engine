import { type Card, createEmptyCard, fsrs } from 'ts-fsrs';

import type { Rating, ScheduleState } from './types';

const scheduler = fsrs({
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

function toCard(state: ScheduleState): Card {
  const { last_review, ...rest } = state;

  if (last_review === null) {
    return {
      ...rest,
      due: new Date(state.due),
      learning_steps: 0,
    };
  }

  return {
    ...rest,
    due: new Date(state.due),
    last_review: new Date(last_review),
    learning_steps: 0,
  };
}

export function next(state: ScheduleState | null, rating: Rating, now: number): ScheduleState {
  const currentCard = state === null ? createEmptyCard(new Date(now)) : toCard(state);

  return toScheduleState(scheduler.next(currentCard, new Date(now), rating).card);
}
