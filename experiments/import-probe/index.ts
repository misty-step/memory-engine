import { State } from 'ts-fsrs';

import type { Prompt, QueueCandidate, ReviewUnitId, ScheduleState } from 'memory-engine/types';

const punctuationTokens = ['.', ',', ';', ':'];

type AuthoredCard = {
  id: string;
  sourceText: string;
  translation: string;
  conceptKey: string;
  stage: 'new' | 'review';
  confidencePrompt: string;
  notes: string[];
};

type AuthoredFixture = {
  id: string;
  domainKey: string;
  sourceKey: string;
  cards: AuthoredCard[];
};

type CompiledSchedule = {
  reviewUnitId: ReviewUnitId;
  state: ScheduleState;
};

export type CompiledImportProbe = {
  fixture: string;
  prompts: Prompt[];
  promptIds: Map<ReviewUnitId, string>;
  queue: QueueCandidate[];
  schedules: CompiledSchedule[];
  productOwnedFields: string[];
  apiGap: string | null;
};

export const latinPrayerFixture: AuthoredFixture = {
  id: 'latin-prayer-authored-v1',
  domainKey: 'latin',
  sourceKey: 'mass-ordinary',
  cards: [
    {
      id: 'credo-in-unum-deum',
      sourceText: 'Credo in unum Deum',
      translation: 'I believe in one God',
      conceptKey: 'creed-opening',
      stage: 'review',
      confidencePrompt: 'How sure are you before revealing the answer?',
      notes: ['Keep confidence outside the kernel until a client proves the need.'],
    },
    {
      id: 'pater-noster',
      sourceText: 'Pater noster',
      translation: 'Our Father',
      conceptKey: 'lords-prayer-opening',
      stage: 'new',
      confidencePrompt: 'How sure are you before revealing the answer?',
      notes: ['Authored taxonomy stays product-owned.'],
    },
  ],
};

function reviewUnitId(value: string): ReviewUnitId {
  return value as ReviewUnitId;
}

function scheduleState(now: number): ScheduleState {
  return {
    due: now - 60_000,
    stability: 4.2,
    difficulty: 3.1,
    elapsed_days: 1,
    scheduled_days: 1,
    reps: 3,
    lapses: 0,
    state: State.Review,
    last_review: now - 86_400_000,
  };
}

export function compileAuthoredFixture(fixture: AuthoredFixture, now: number): CompiledImportProbe {
  const prompts: Prompt[] = [];
  const promptIds = new Map<ReviewUnitId, string>();
  const queue: QueueCandidate[] = [];
  const schedules: CompiledSchedule[] = [];

  for (const card of fixture.cards) {
    const unitId = reviewUnitId(`import-${card.id}`);
    const promptId = `${card.id}-translation`;

    prompts.push({
      kind: 'shortAnswer',
      reviewUnitId: unitId,
      prompt: `Translate: ${card.sourceText}`,
      acceptedAnswers: [card.translation],
      equivalenceGroups: [['God', 'god']],
      ignoredTokens: punctuationTokens,
    });
    promptIds.set(unitId, promptId);

    const existingSchedule = card.stage === 'review' ? scheduleState(now) : null;
    queue.push({
      reviewUnitId: unitId,
      scheduleState: existingSchedule,
      due: existingSchedule?.due ?? now - 60_000,
      progression: null,
      conceptKey: card.conceptKey,
      sourceKey: fixture.sourceKey,
      domainKey: fixture.domainKey,
    });

    if (existingSchedule !== null) {
      schedules.push({ reviewUnitId: unitId, state: existingSchedule });
    }
  }

  return {
    fixture: fixture.id,
    prompts,
    promptIds,
    queue,
    schedules,
    productOwnedFields: ['sourceText', 'translation', 'confidencePrompt', 'notes'],
    apiGap: null,
  };
}

if (import.meta.main) {
  const compiled = compileAuthoredFixture(latinPrayerFixture, Date.now());
  console.log(
    JSON.stringify(
      {
        fixture: compiled.fixture,
        prompts: compiled.prompts.length,
        queueCandidates: compiled.queue.length,
        schedules: compiled.schedules.length,
        productOwnedFields: compiled.productOwnedFields,
        apiGap: compiled.apiGap,
      },
      null,
      2,
    ),
  );
}
