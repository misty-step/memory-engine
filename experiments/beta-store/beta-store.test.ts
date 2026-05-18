import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { describe, expect, test } from 'bun:test';
import { State } from 'ts-fsrs';

import { createMemoryService } from '../../service';
import { type Prompt, Rating, type ReviewUnitId, type ScheduleState } from '../../src';
import {
  DuplicateAppliedReviewError,
  type GeneratedPromptDraft,
  createBetaPersistenceStore,
} from './index';

const now = Date.UTC(2026, 4, 18, 12, 0, 0);

function reviewUnitId(value: string): ReviewUnitId {
  return value as ReviewUnitId;
}

function scheduleState(overrides: Partial<ScheduleState> = {}): ScheduleState {
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
    ...overrides,
  };
}

function shortAnswerPrompt(unitId: ReviewUnitId, prompt = 'Translate: Pater noster'): Prompt {
  return {
    kind: 'shortAnswer',
    reviewUnitId: unitId,
    prompt,
    acceptedAnswers: ['Our Father'],
    equivalenceGroups: [],
    ignoredTokens: [],
  };
}

async function withTempStore<T>(run: (path: string) => Promise<T>): Promise<T> {
  const directory = await mkdtemp(join(tmpdir(), 'memory-engine-beta-store-'));
  try {
    return await run(join(directory, 'store.json'));
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}

describe('beta persistence store', () => {
  test('persists source material, references, drafts, review state, attempts, and queue across reload', async () => {
    await withTempStore(async (path) => {
      const store = await createBetaPersistenceStore(path);
      const source = await store.saveSourceDocument({
        id: 'src-latin-prayer',
        kind: 'text',
        title: 'Latin prayer note',
        body: 'Pater noster means Our Father.',
        uri: null,
        permission: 'model-eligible',
        freshness: now,
        createdAt: now,
      });
      const reference = await store.saveReferenceSpan({
        id: 'ref-pater',
        sourceDocumentId: source.id,
        label: 'Pater noster translation',
        text: 'Pater noster means Our Father.',
        locator: 'paragraph:1',
        createdAt: now,
      });
      const draft = await store.saveGeneratedPromptDraft({
        id: 'draft-pater',
        sourceDocumentIds: [source.id],
        referenceSpanIds: [reference.id],
        generationRunId: 'run-pater',
        reviewUnitId: reviewUnitId('beta-pater-noster'),
        promptId: 'pater-translation',
        prompt: shortAnswerPrompt(reviewUnitId('beta-pater-noster')),
        queue: {
          reviewUnitId: reviewUnitId('beta-pater-noster'),
          due: now - 60_000,
          progression: null,
          conceptKey: 'lords-prayer-opening',
          sourceKey: 'latin-prayer-note',
          domainKey: 'latin',
        },
        model: {
          provider: 'fixture',
          name: 'deterministic-draft',
          version: 'v1',
        },
        validation: {
          status: 'accepted',
          reasons: [],
        },
        critiqueNotes: ['Grounded in the cited source span.'],
        createdAt: now,
      });
      await store.saveGenerationRun({
        id: 'run-pater',
        sourceDocumentIds: [source.id],
        draftIds: [draft.id],
        provider: 'fixture',
        model: 'deterministic-draft',
        startedAt: now - 1_000,
        completedAt: now,
        validationFailures: [],
      });
      await store.approveGeneratedPromptDraft(draft.id, {
        initialScheduleState: scheduleState({ reps: 2, state: State.Review }),
      });

      const service = createMemoryService({
        store,
        now: () => now,
        masteryPolicy: (schedule) => schedule.state === State.Review && schedule.reps >= 3,
      });

      const review = await service.execute({
        kind: 'grade/apply-review',
        prompt: draft.prompt,
        promptId: draft.promptId,
        submittedAnswer: 'Our Father',
        responseTimeMs: 1_800,
        occurredAt: now,
      });

      expect(review.grade).toMatchObject({
        verdict: 'correct',
        rating: Rating.Good,
        isCorrect: true,
      });
      expect(review.scheduleState.reps).toBe(3);

      const reloaded = await createBetaPersistenceStore(path);
      const snapshot = reloaded.snapshot();
      const queue = await reloaded.listQueueCandidates();

      expect(snapshot.sourceDocuments).toEqual([source]);
      expect(snapshot.referenceSpans).toEqual([reference]);
      expect(snapshot.generatedPromptDrafts).toEqual([draft]);
      expect(snapshot.generationRuns).toHaveLength(1);
      expect(snapshot.attempts).toEqual([review.attempt]);
      expect(snapshot.schedules).toEqual([
        {
          reviewUnitId: reviewUnitId('beta-pater-noster'),
          state: review.scheduleState,
        },
      ]);
      expect(queue).toEqual([
        {
          reviewUnitId: reviewUnitId('beta-pater-noster'),
          scheduleState: review.scheduleState,
          due: review.scheduleState.due,
          progression: null,
          conceptKey: 'lords-prayer-opening',
          sourceKey: 'latin-prayer-note',
          domainKey: 'latin',
        },
      ]);
    });
  });

  test('rejects duplicate applied reviews and failed commits without corrupting schedule history', async () => {
    await withTempStore(async (path) => {
      const unitId = reviewUnitId('beta-duplicate-safe');
      const prompt = shortAnswerPrompt(unitId);
      const store = await createBetaPersistenceStore(path);
      await store.saveReviewUnit({
        reviewUnitId: unitId,
        promptId: 'duplicate-safe-prompt',
        prompt,
        queue: {
          reviewUnitId: unitId,
          due: now - 60_000,
          progression: null,
          conceptKey: 'duplicate-safety',
          sourceKey: 'beta-fixture',
          domainKey: 'qa',
        },
        referenceSpanIds: [],
        generatedPromptDraftId: null,
        createdAt: now,
      });

      const service = createMemoryService({
        store,
        now: () => now,
        masteryPolicy: (schedule) => schedule.state === State.Review && schedule.reps >= 3,
      });

      const firstReview = await service.execute({
        kind: 'grade/apply-review',
        prompt,
        promptId: 'duplicate-safe-prompt',
        submittedAnswer: 'Our Father',
        responseTimeMs: 1_800,
        occurredAt: now,
      });

      await expect(
        service.execute({
          kind: 'grade/apply-review',
          prompt,
          promptId: 'duplicate-safe-prompt',
          submittedAnswer: 'Our Father',
          responseTimeMs: 1_800,
          occurredAt: now,
        }),
      ).rejects.toBeInstanceOf(DuplicateAppliedReviewError);

      expect(store.snapshot().attempts).toEqual([firstReview.attempt]);
      expect(store.snapshot().schedules).toEqual([
        {
          reviewUnitId: unitId,
          state: firstReview.scheduleState,
        },
      ]);

      store.failNextCommitForTest();
      await expect(
        service.execute({
          kind: 'grade/apply-review',
          prompt,
          promptId: 'duplicate-safe-prompt',
          submittedAnswer: 'Our Father',
          responseTimeMs: 2_100,
          occurredAt: now + 1_000,
        }),
      ).rejects.toThrow('Injected beta store commit failure');

      const reloaded = await createBetaPersistenceStore(path);
      expect(reloaded.snapshot().attempts).toEqual([firstReview.attempt]);
      expect(reloaded.snapshot().schedules).toEqual([
        {
          reviewUnitId: unitId,
          state: firstReview.scheduleState,
        },
      ]);
    });
  });

  test('reloads a realistic uneven queue pile with progression, prerequisites, supersession, and anti-clumping metadata', async () => {
    await withTempStore(async (path) => {
      const store = await createBetaPersistenceStore(path);
      const stageOne = reviewUnitId('catechism-commandments-worked-example');
      const stageTwo = reviewUnitId('catechism-commandments-free-recall');
      const dueReview = reviewUnitId('latin-psalm-due-review');
      const sameSource = reviewUnitId('latin-psalm-same-source');
      const superseded = reviewUnitId('catechism-commandments-superseded');

      await store.saveReviewUnit({
        reviewUnitId: stageOne,
        promptId: 'commandments-worked',
        prompt: shortAnswerPrompt(stageOne, 'What is the first commandment?'),
        queue: {
          reviewUnitId: stageOne,
          due: now + 86_400_000,
          progression: {
            progressionGroup: 'ten-commandments',
            stageOrder: 1,
            requires: [],
            supersedes: [superseded],
          },
          conceptKey: 'first-commandment',
          sourceKey: 'catechism',
          domainKey: 'theology',
        },
        referenceSpanIds: [],
        generatedPromptDraftId: null,
        createdAt: now,
      });
      await store.setScheduleState(stageOne, scheduleState({ due: now + 86_400_000, reps: 4 }));
      await store.saveReviewUnit({
        reviewUnitId: stageTwo,
        promptId: 'commandments-recall',
        prompt: shortAnswerPrompt(stageTwo, 'Recall the first commandment.'),
        queue: {
          reviewUnitId: stageTwo,
          due: now - 300_000,
          progression: {
            progressionGroup: 'ten-commandments',
            stageOrder: 2,
            requires: [stageOne],
            supersedes: [stageOne, superseded],
          },
          conceptKey: 'first-commandment',
          sourceKey: 'catechism',
          domainKey: 'theology',
        },
        referenceSpanIds: [],
        generatedPromptDraftId: null,
        createdAt: now,
      });
      await store.saveReviewUnit({
        reviewUnitId: dueReview,
        promptId: 'psalm-due',
        prompt: shortAnswerPrompt(dueReview, 'Translate the psalm phrase.'),
        queue: {
          reviewUnitId: dueReview,
          due: now - 3_600_000,
          progression: null,
          conceptKey: 'latin-psalm',
          sourceKey: 'psalm-notes',
          domainKey: 'latin',
        },
        referenceSpanIds: [],
        generatedPromptDraftId: null,
        createdAt: now,
      });
      await store.setScheduleState(
        dueReview,
        scheduleState({ due: now - 3_600_000, reps: 5, scheduled_days: 5 }),
      );
      await store.saveReviewUnit({
        reviewUnitId: sameSource,
        promptId: 'psalm-same-source',
        prompt: shortAnswerPrompt(sameSource, 'Translate another psalm phrase.'),
        queue: {
          reviewUnitId: sameSource,
          due: now - 3_500_000,
          progression: null,
          conceptKey: 'latin-psalm',
          sourceKey: 'psalm-notes',
          domainKey: 'latin',
        },
        referenceSpanIds: [],
        generatedPromptDraftId: null,
        createdAt: now,
      });
      await store.setScheduleState(
        sameSource,
        scheduleState({ due: now - 3_500_000, reps: 5, scheduled_days: 5 }),
      );
      await store.saveReviewUnit({
        reviewUnitId: superseded,
        promptId: 'commandments-superseded',
        prompt: shortAnswerPrompt(superseded, 'Choose the first commandment.'),
        queue: {
          reviewUnitId: superseded,
          due: now - 60_000,
          progression: {
            progressionGroup: 'ten-commandments',
            stageOrder: 0,
            requires: [],
            supersedes: [],
          },
          conceptKey: 'first-commandment',
          sourceKey: 'catechism',
          domainKey: 'theology',
        },
        referenceSpanIds: [],
        generatedPromptDraftId: null,
        createdAt: now,
      });

      const reloaded = await createBetaPersistenceStore(path);
      const service = createMemoryService({
        store: reloaded,
        now: () => now,
        masteryPolicy: (schedule) => schedule.state === State.Review && schedule.reps >= 3,
      });
      const queue = await reloaded.listQueueCandidates();
      const next = await service.execute({
        kind: 'next-queue',
        options: {
          recentCandidates: [
            {
              reviewUnitId: dueReview,
              scheduleState: scheduleState({ due: now - 3_600_000, reps: 5 }),
              due: now - 3_600_000,
              progression: null,
              conceptKey: 'latin-psalm',
              sourceKey: 'psalm-notes',
              domainKey: 'latin',
            },
          ],
        },
      });

      expect(queue).toHaveLength(5);
      expect(queue.find((candidate) => candidate.reviewUnitId === stageTwo)).toMatchObject({
        progression: {
          progressionGroup: 'ten-commandments',
          stageOrder: 2,
          requires: [stageOne],
          supersedes: [stageOne, superseded],
        },
      });
      expect(next.candidate?.reviewUnitId).toBe(dueReview);
    });
  });

  test('validates generated drafts before promotion to review units', async () => {
    await withTempStore(async (path) => {
      const store = await createBetaPersistenceStore(path);
      await store.saveSourceDocument({
        id: 'src-generated',
        kind: 'text',
        title: 'Generated source',
        body: 'A cited source.',
        uri: null,
        permission: 'local-only',
        freshness: null,
        createdAt: now,
      });
      await store.saveReferenceSpan({
        id: 'ref-generated',
        sourceDocumentId: 'src-generated',
        label: 'Cited sentence',
        text: 'A cited source.',
        locator: 'line:1',
        createdAt: now,
      });
      const rejectedDraft: GeneratedPromptDraft = {
        id: 'draft-rejected',
        sourceDocumentIds: ['src-generated'],
        referenceSpanIds: ['ref-generated'],
        generationRunId: null,
        reviewUnitId: reviewUnitId('rejected-unit'),
        promptId: 'rejected-prompt',
        prompt: shortAnswerPrompt(reviewUnitId('rejected-unit')),
        queue: {
          reviewUnitId: reviewUnitId('rejected-unit'),
          due: now,
          progression: null,
          conceptKey: 'generated',
          sourceKey: 'source',
          domainKey: 'qa',
        },
        model: {
          provider: 'fixture',
          name: 'deterministic-draft',
          version: 'v1',
        },
        validation: {
          status: 'rejected',
          reasons: ['Unsupported claim.'],
        },
        critiqueNotes: ['No supporting quote.'],
        createdAt: now,
      };

      await store.saveGeneratedPromptDraft(rejectedDraft);
      await expect(store.approveGeneratedPromptDraft(rejectedDraft.id)).rejects.toThrow(
        'Only accepted generated prompt drafts can be approved',
      );

      await expect(
        store.saveGeneratedPromptDraft({
          ...rejectedDraft,
          id: 'draft-missing-reference',
          validation: { status: 'accepted', reasons: [] },
          referenceSpanIds: ['missing-reference'],
        }),
      ).rejects.toThrow('Unknown reference span: missing-reference');

      await store.saveGeneratedPromptDraft({
        ...rejectedDraft,
        id: 'draft-missing-generation-run',
        reviewUnitId: reviewUnitId('accepted-without-run'),
        promptId: 'accepted-without-run-prompt',
        prompt: shortAnswerPrompt(reviewUnitId('accepted-without-run')),
        queue: {
          reviewUnitId: reviewUnitId('accepted-without-run'),
          due: now,
          progression: null,
          conceptKey: 'generated',
          sourceKey: 'source',
          domainKey: 'qa',
        },
        validation: { status: 'accepted', reasons: [] },
      });
      await expect(
        store.approveGeneratedPromptDraft('draft-missing-generation-run'),
      ).rejects.toThrow(
        'Accepted generated prompt drafts require a generation run before approval',
      );
    });
  });
});
