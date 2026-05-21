import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { describe, expect, test } from 'bun:test';

import { createBetaPersistenceStore } from '../beta-store';
import { runBetaGeneration } from './index';

const now = Date.UTC(2026, 4, 20, 12, 0, 0);

async function withTempStore<T>(run: (path: string) => Promise<T>): Promise<T> {
  const directory = await mkdtemp(join(tmpdir(), 'memory-engine-beta-generation-'));
  try {
    return await run(join(directory, 'store.json'));
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}

describe('beta generation probe', () => {
  test('generates accepted quiz and exercise drafts with provenance and promotes them', async () => {
    await withTempStore(async (path) => {
      const store = await createBetaPersistenceStore(path);
      await store.saveSourceDocument({
        id: 'src-nato',
        kind: 'text',
        title: 'NATO phonetic alphabet notes',
        body: [
          'Concept: NATO letter A',
          'Activity: quiz',
          'Stage: recognition-3',
          'Question: What is the NATO phonetic alphabet word for A?',
          'Answer: ALFA',
          'Distractors: BRAVO, CHARLIE',
          'Reference: The NATO phonetic alphabet word for A is ALFA.',
          '',
          'Concept: NATO CAT composition',
          'Activity: exercise',
          'Stage: composition',
          'Question: Spell CAT over the phone using the NATO phonetic alphabet.',
          'Answer: CHARLIE ALFA TANGO',
          'Worked Solution: C is CHARLIE, A is ALFA, and T is TANGO.',
          'Reference: C is CHARLIE. A is ALFA. T is TANGO.',
        ].join('\n'),
        uri: null,
        permission: 'model-eligible',
        freshness: now,
        createdAt: now,
      });

      const result = await runBetaGeneration(store, {
        runId: 'run-nato',
        sourceDocumentIds: ['src-nato'],
        startedAt: now,
        completedAt: now + 1_000,
        defaultDue: now - 60_000,
      });

      expect(result).toEqual({
        runId: 'run-nato',
        draftIds: [
          'run-nato-draft-src-nato-1-nato-letter-a',
          'run-nato-draft-src-nato-2-nato-cat-composition',
        ],
        acceptedDraftIds: [
          'run-nato-draft-src-nato-1-nato-letter-a',
          'run-nato-draft-src-nato-2-nato-cat-composition',
        ],
        rejectedDraftIds: [],
        validationFailures: [],
      });

      const snapshot = store.snapshot();
      expect(snapshot.referenceSpans).toHaveLength(2);
      expect(snapshot.generationRuns).toEqual([
        {
          id: 'run-nato',
          sourceDocumentIds: ['src-nato'],
          draftIds: result.draftIds,
          provider: 'fixture',
          model: 'deterministic-beta-generator',
          startedAt: now,
          completedAt: now + 1_000,
          validationFailures: [],
        },
      ]);
      expect(snapshot.generatedPromptDrafts).toMatchObject([
        {
          id: 'run-nato-draft-src-nato-1-nato-letter-a',
          activityKind: 'quiz',
          activityStage: 'recognition-3',
          prompt: {
            kind: 'mcq',
            prompt: 'What is the NATO phonetic alphabet word for A?',
            choices: ['ALFA', 'BRAVO', 'CHARLIE'],
            correctChoice: 'ALFA',
          },
          validation: { status: 'accepted', reasons: [] },
        },
        {
          id: 'run-nato-draft-src-nato-2-nato-cat-composition',
          activityKind: 'exercise',
          activityStage: 'composition',
          workedSolution: 'C is CHARLIE, A is ALFA, and T is TANGO.',
          prompt: {
            kind: 'recitation',
            prompt: 'Spell CAT over the phone using the NATO phonetic alphabet.',
            acceptedAnswers: ['CHARLIE ALFA TANGO'],
          },
          validation: { status: 'accepted', reasons: [] },
        },
      ]);

      const reviewUnit = await store.approveGeneratedPromptDraft(
        'run-nato-draft-src-nato-2-nato-cat-composition',
      );
      const queue = await store.listQueueCandidates();

      expect(reviewUnit.generatedPromptDraftId).toBe(
        'run-nato-draft-src-nato-2-nato-cat-composition',
      );
      expect(queue).toContainEqual(
        expect.objectContaining({
          reviewUnitId: reviewUnit.reviewUnitId,
          conceptKey: 'nato-cat-composition',
          progression: expect.objectContaining({
            progressionGroup: 'nato-cat-composition',
            stageOrder: 4,
          }),
        }),
      );
    });
  });

  test('persists rejected unsupported and duplicate-ish drafts with critique reasons', async () => {
    await withTempStore(async (path) => {
      const store = await createBetaPersistenceStore(path);
      await store.saveSourceDocument({
        id: 'src-options',
        kind: 'text',
        title: 'Options notes',
        body: [
          'Concept: Gamma definition',
          'Activity: quiz',
          'Stage: free-recall',
          'Question: What does Gamma measure?',
          'Answer: The rate of change of Delta.',
          'Reference: Gamma measures the rate of change of Delta.',
          '',
          'Concept: Gamma definition',
          'Activity: quiz',
          'Stage: free-recall',
          'Question: What does Gamma measure?',
          'Answer: The rate of change of Delta.',
          'Reference: Gamma measures the rate of change of Delta.',
          '',
          'Concept: Gamma advice',
          'Activity: exercise',
          'Stage: composition',
          'Question: Should I buy this options position?',
          'Answer: Buy the position.',
          'Worked Solution: This would be personalized financial advice.',
          'Reference: Gamma measures convexity, not whether a person should trade.',
          'Unsupported: true',
        ].join('\n'),
        uri: null,
        permission: 'local-only',
        freshness: now,
        createdAt: now,
      });

      const result = await runBetaGeneration(store, {
        runId: 'run-options',
        sourceDocumentIds: ['src-options'],
        startedAt: now,
        defaultDue: now,
      });

      expect(result.acceptedDraftIds).toEqual(['run-options-draft-src-options-1-gamma-definition']);
      expect(result.rejectedDraftIds).toEqual([
        'run-options-draft-src-options-2-gamma-definition',
        'run-options-draft-src-options-3-gamma-advice',
      ]);
      expect(store.snapshot().generatedPromptDrafts).toMatchObject([
        {
          id: 'run-options-draft-src-options-1-gamma-definition',
          validation: { status: 'accepted', reasons: [] },
        },
        {
          id: 'run-options-draft-src-options-2-gamma-definition',
          validation: { status: 'rejected', reasons: ['Duplicate-ish generated draft'] },
          critiqueNotes: ['Rejected: Duplicate-ish generated draft'],
        },
        {
          id: 'run-options-draft-src-options-3-gamma-advice',
          activityKind: 'exercise',
          validation: {
            status: 'rejected',
            reasons: ['Unsupported by cited source material'],
          },
        },
      ]);
    });
  });

  test('records missing provenance failures without saving malformed drafts', async () => {
    await withTempStore(async (path) => {
      const store = await createBetaPersistenceStore(path);
      await store.saveSourceDocument({
        id: 'src-missing-provenance',
        kind: 'text',
        title: 'Unsupported note',
        body: [
          'Concept: unsupported',
          'Activity: quiz',
          'Question: What is unsupported?',
          'Answer: This has no cited source span.',
        ].join('\n'),
        uri: null,
        permission: 'local-only',
        freshness: null,
        createdAt: now,
      });

      const result = await runBetaGeneration(store, {
        runId: 'run-missing-provenance',
        sourceDocumentIds: ['src-missing-provenance'],
        startedAt: now,
        defaultDue: now,
      });

      expect(result).toEqual({
        runId: 'run-missing-provenance',
        draftIds: [],
        acceptedDraftIds: [],
        rejectedDraftIds: [],
        validationFailures: [
          'src-missing-provenance block 1: generated drafts require source provenance',
        ],
      });
      expect(store.snapshot().generatedPromptDrafts).toEqual([]);
      expect(store.snapshot().generationRuns).toMatchObject([
        {
          id: 'run-missing-provenance',
          draftIds: [],
          validationFailures: [
            'src-missing-provenance block 1: generated drafts require source provenance',
          ],
        },
      ]);
    });
  });

  test('keeps generated ids distinct across multiple source documents', async () => {
    await withTempStore(async (path) => {
      const store = await createBetaPersistenceStore(path);
      for (const sourceId of ['src-a', 'src-b']) {
        await store.saveSourceDocument({
          id: sourceId,
          kind: 'text',
          title: `Gamma note ${sourceId}`,
          body: [
            'Concept: Gamma definition',
            'Activity: quiz',
            'Stage: free-recall',
            `Question: What does Gamma measure in ${sourceId}?`,
            'Answer: The rate of change of Delta.',
            `Reference: ${sourceId} says Gamma measures the rate of change of Delta.`,
          ].join('\n'),
          uri: null,
          permission: 'model-eligible',
          freshness: now,
          createdAt: now,
        });
      }

      const result = await runBetaGeneration(store, {
        runId: 'run-multi-source',
        sourceDocumentIds: ['src-a', 'src-b'],
        startedAt: now,
        defaultDue: now,
      });

      expect(result.draftIds).toEqual([
        'run-multi-source-draft-src-a-1-gamma-definition',
        'run-multi-source-draft-src-b-1-gamma-definition',
      ]);
      expect(store.snapshot().referenceSpans.map((span) => span.id)).toEqual([
        'run-multi-source-ref-src-a-1-gamma-definition',
        'run-multi-source-ref-src-b-1-gamma-definition',
      ]);
      expect(
        store.snapshot().generatedPromptDrafts.map((draft) => String(draft.reviewUnitId)),
      ).toEqual([
        'generated-quiz-src-a-1-gamma-definition',
        'generated-quiz-src-b-1-gamma-definition',
      ]);
    });
  });
});
