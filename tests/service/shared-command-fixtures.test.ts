import { describe, expect, test } from 'bun:test';
import { State } from 'ts-fsrs';

import serviceScenarios from '../../fixtures/service-command-scenarios.json';
import {
  type MemoryServiceCommand,
  type MemoryServiceResult,
  type MemoryServiceStore,
  type ServiceAttemptRecord,
  createMemoryService,
} from '../../service';
import type { MasteryPolicy, Prompt, QueueCandidate, ReviewUnitId, ScheduleState } from '../../src';

type ScenarioUnitFixture = {
  reviewUnitId: ReviewUnitId;
  promptId: string;
  prompt: Prompt;
  queue: Omit<QueueCandidate, 'reviewUnitId' | 'scheduleState' | 'due'>;
  scheduleState: ScheduleState | null;
};

type ExpectedResult = {
  kind: MemoryServiceResult['kind'];
  attempt?: Partial<ServiceAttemptRecord>;
  grade?: {
    verdict: string;
    rating: number;
    isCorrect: boolean;
    expectedAnswer: string;
  };
  scheduleState?: ScheduleState;
  candidate?: QueueCandidate | null;
};

type ServiceScenarioFixture = {
  name: string;
  now: number;
  units: ScenarioUnitFixture[];
  commands: MemoryServiceCommand[];
  expected: ExpectedResult[];
};

const masteryPolicy: MasteryPolicy<ScheduleState> = (scheduleState) => {
  return scheduleState.state === State.Review && scheduleState.reps >= 3;
};

describe('shared service command parity fixtures', () => {
  const scenarios = serviceScenarios as unknown as ServiceScenarioFixture[];

  test.each(scenarios)('$name', async (scenario) => {
    expect(scenario.commands).toHaveLength(scenario.expected.length);

    const { schedules, store } = createSharedFixtureStore(scenario);
    const service = createMemoryService({
      store,
      now: () => scenario.now,
      masteryPolicy,
    });

    for (const [index, command] of scenario.commands.entries()) {
      const actual = await service.execute(command);
      const expected = scenario.expected[index];

      if (expected === undefined) {
        throw new Error(`${scenario.name} step ${index} is missing an expected result`);
      }

      assertExpectedResult(actual, expected);
    }

    for (const expected of scenario.expected) {
      if (expected.kind === 'review-applied' && expected.scheduleState !== undefined) {
        const reviewUnitId = expected.attempt?.reviewUnitId;
        expect(reviewUnitId).toBeDefined();
        expect(schedules.get(reviewUnitId as ReviewUnitId)).toEqual(expected.scheduleState);
      }
    }
  });
});

function createSharedFixtureStore(scenario: ServiceScenarioFixture): {
  schedules: Map<ReviewUnitId, ScheduleState>;
  store: MemoryServiceStore;
} {
  const units = new Map(scenario.units.map((unit) => [unit.reviewUnitId, unit]));
  const schedules = new Map<ReviewUnitId, ScheduleState>(
    scenario.units.flatMap((unit) =>
      unit.scheduleState === null ? [] : [[unit.reviewUnitId, unit.scheduleState]],
    ),
  );

  function assertKnownReviewUnit(reviewUnitId: ReviewUnitId): void {
    if (!units.has(reviewUnitId)) {
      throw new Error(`Unknown review unit: ${reviewUnitId}`);
    }
  }

  function assertAttemptContract(attempt: ServiceAttemptRecord): void {
    assertKnownReviewUnit(attempt.reviewUnitId);

    if (attempt.submittedAnswer.trim() === '') {
      throw new Error('Attempt answer must not be blank');
    }

    if (!Number.isInteger(attempt.responseTimeMs) || attempt.responseTimeMs <= 0) {
      throw new Error('Attempt response time must be a positive integer');
    }

    if (!Number.isInteger(attempt.occurredAt)) {
      throw new Error('Attempt timestamp must be an integer epoch millisecond value');
    }
  }

  const store: MemoryServiceStore = {
    async recordAttempt(attempt: ServiceAttemptRecord): Promise<void> {
      assertAttemptContract(attempt);
    },
    async readScheduleState(reviewUnitId: ReviewUnitId): Promise<ScheduleState | null> {
      assertKnownReviewUnit(reviewUnitId);
      return schedules.get(reviewUnitId) ?? null;
    },
    async applyReview(
      reviewUnitId: ReviewUnitId,
      attempt: ServiceAttemptRecord,
      nextScheduleState: ScheduleState,
    ): Promise<void> {
      assertKnownReviewUnit(reviewUnitId);

      if (reviewUnitId !== attempt.reviewUnitId) {
        throw new Error('Applied review unit must match the attempt review unit');
      }

      if (nextScheduleState.last_review !== attempt.occurredAt) {
        throw new Error('Schedule last_review must match the attempt timestamp');
      }

      assertAttemptContract(attempt);
      schedules.set(reviewUnitId, nextScheduleState);
    },
    async listQueueCandidates(): Promise<QueueCandidate[]> {
      return scenario.units.map((unit) => {
        const currentScheduleState = schedules.get(unit.reviewUnitId) ?? null;

        return {
          reviewUnitId: unit.reviewUnitId,
          scheduleState: currentScheduleState,
          due: currentScheduleState?.due ?? scenario.now - 60_000,
          ...unit.queue,
        };
      });
    },
  };

  return { schedules, store };
}

function assertExpectedResult(actual: MemoryServiceResult, expected: ExpectedResult): void {
  expect(actual.kind).toBe(expected.kind);

  switch (expected.kind) {
    case 'attempt-recorded': {
      expect(actual.kind).toBe('attempt-recorded');
      if (actual.kind !== 'attempt-recorded') {
        return;
      }
      expect(actual.attempt).toMatchObject(expected.attempt ?? {});
      return;
    }
    case 'review-applied': {
      expect(actual.kind).toBe('review-applied');
      if (actual.kind !== 'review-applied') {
        return;
      }
      if (expected.scheduleState === undefined) {
        throw new Error('review-applied fixture must include scheduleState');
      }
      expect(actual.attempt).toMatchObject(expected.attempt ?? {});
      expect(actual.grade).toMatchObject(expected.grade ?? {});
      expect(actual.scheduleState).toEqual(expected.scheduleState);
      return;
    }
    case 'queue-selected': {
      expect(actual.kind).toBe('queue-selected');
      if (actual.kind !== 'queue-selected') {
        return;
      }
      if (expected.candidate === undefined) {
        throw new Error('queue-selected fixture must include candidate');
      }
      expect(actual.candidate).toEqual(expected.candidate);
      return;
    }
  }
}
