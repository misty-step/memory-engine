import { describe, expect, test } from 'bun:test';
import { type Card, type CardInput, State } from 'ts-fsrs';

import type { ScheduleState } from '../../src';

type Assert<T extends true> = T;
type Equal<Left, Right> = (<Value>() => Value extends Left ? 1 : 2) extends <
  Value,
>() => Value extends Right ? 1 : 2
  ? true
  : false;

type SharedCardFields = Omit<Card, 'due' | 'last_review' | 'learning_steps'>;
type _ScheduleStatePreservesTsFsrsCardFields = Assert<
  Equal<Omit<ScheduleState, 'due' | 'last_review'>, SharedCardFields>
>;
type _ScheduleStateUsesNumericDue = Assert<Equal<ScheduleState['due'], number>>;
type _ScheduleStateUsesNullableNumericLastReview = Assert<
  Equal<ScheduleState['last_review'], number | null>
>;

function acceptsCardInput(_state: CardInput): void {}

describe('ScheduleState', () => {
  test('is the JSON-safe ts-fsrs card shape without transient learning steps', () => {
    const state: ScheduleState = {
      due: 0,
      stability: 0,
      difficulty: 0,
      elapsed_days: 0,
      scheduled_days: 0,
      reps: 0,
      lapses: 0,
      state: State.New,
      last_review: null,
    };

    acceptsCardInput({ ...state, learning_steps: 0 });

    expect(state.due).toBe(0);
    expect(state.last_review).toBeNull();
  });
});
