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
            choices: ['CHARLIE', 'ALFA', 'BRAVO'],
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

  test('generates a shared NATO activity ladder with shuffled recognition choices and ordered variants', async () => {
    await withTempStore(async (path) => {
      const store = await createBetaPersistenceStore(path);
      await store.saveSourceDocument({
        id: 'src-nato-ladder',
        kind: 'text',
        title: 'NATO ladder notes',
        body: [
          'Concept: NATO phonetic alphabet',
          'Activity: quiz',
          'Stage: recognition-3',
          'Question: A is which NATO phonetic alphabet word?',
          'Answer: ALFA',
          'Distractors: BRAVO, CHARLIE',
          'Reference: A is ALFA in the NATO phonetic alphabet.',
          '',
          'Concept: NATO phonetic alphabet',
          'Activity: quiz',
          'Stage: recognition-5',
          'Question: A is which NATO phonetic alphabet word among these five choices?',
          'Answer: ALFA',
          'Distractors: BRAVO, CHARLIE, DELTA, ECHO',
          'Reference: A is ALFA in the NATO phonetic alphabet.',
          '',
          'Concept: NATO phonetic alphabet',
          'Activity: quiz',
          'Stage: typed-recall',
          'Question: Type the NATO phonetic alphabet word for A.',
          'Answer: ALFA',
          'Reference: A is ALFA in the NATO phonetic alphabet.',
          '',
          'Concept: NATO phonetic alphabet',
          'Activity: exercise',
          'Stage: composition',
          'Question: Spell CAT over the phone using the NATO phonetic alphabet.',
          'Answer: CHARLIE ALFA TANGO',
          'Worked Solution: C is CHARLIE, A is ALFA, and T is TANGO.',
          'Scoring Rubric: Pass only when each letter is converted to the matching NATO word in order.',
          'Reference: C is CHARLIE. A is ALFA. T is TANGO.',
        ].join('\n'),
        uri: null,
        permission: 'model-eligible',
        freshness: now,
        createdAt: now,
      });

      const result = await runBetaGeneration(store, {
        runId: 'run-nato-ladder',
        sourceDocumentIds: ['src-nato-ladder'],
        startedAt: now,
        completedAt: now,
        defaultDue: now - 60_000,
      });

      expect(result.acceptedDraftIds).toEqual([
        'run-nato-ladder-draft-src-nato-ladder-1-nato-phonetic-alphabet',
        'run-nato-ladder-draft-src-nato-ladder-2-nato-phonetic-alphabet',
        'run-nato-ladder-draft-src-nato-ladder-3-nato-phonetic-alphabet',
        'run-nato-ladder-draft-src-nato-ladder-4-nato-phonetic-alphabet',
      ]);

      const drafts = store.snapshot().generatedPromptDrafts;
      expect(drafts.map((draft) => draft.queue.conceptKey)).toEqual([
        'nato-phonetic-alphabet',
        'nato-phonetic-alphabet',
        'nato-phonetic-alphabet',
        'nato-phonetic-alphabet',
      ]);
      expect(drafts.map((draft) => draft.queue.progression?.progressionGroup)).toEqual([
        'nato-phonetic-alphabet',
        'nato-phonetic-alphabet',
        'nato-phonetic-alphabet',
        'nato-phonetic-alphabet',
      ]);
      expect(drafts.map((draft) => draft.queue.progression?.stageOrder)).toEqual([1, 2, 3, 4]);
      expect(drafts.map((draft) => draft.queue.progression?.requires.map(String))).toEqual([
        [],
        ['generated-quiz-src-nato-ladder-1-nato-phonetic-alphabet'],
        ['generated-quiz-src-nato-ladder-2-nato-phonetic-alphabet'],
        ['generated-quiz-src-nato-ladder-3-nato-phonetic-alphabet'],
      ]);

      expect(drafts[0]?.prompt).toMatchObject({
        kind: 'mcq',
        choices: ['CHARLIE', 'ALFA', 'BRAVO'],
        correctChoice: 'ALFA',
      });
      expect(drafts[1]?.prompt).toMatchObject({
        kind: 'mcq',
        choices: ['ECHO', 'DELTA', 'CHARLIE', 'ALFA', 'BRAVO'],
        correctChoice: 'ALFA',
      });
      expect(drafts[2]?.prompt).toMatchObject({
        kind: 'shortAnswer',
        acceptedAnswers: ['ALFA'],
      });
      expect(drafts[3]).toMatchObject({
        activityKind: 'exercise',
        workedSolution: 'C is CHARLIE, A is ALFA, and T is TANGO.',
        scoringRubric:
          'Pass only when each letter is converted to the matching NATO word in order.',
        prompt: {
          kind: 'recitation',
          acceptedAnswers: ['CHARLIE ALFA TANGO'],
        },
      });
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

  test('compiles arbitrary prose into source-grounded quiz and exercise drafts', async () => {
    await withTempStore(async (path) => {
      const store = await createBetaPersistenceStore(path);
      await store.saveSourceDocument({
        id: 'src-plain-notes',
        kind: 'text',
        title: 'Greek notes',
        body: [
          'Alpha is the first Greek letter.',
          'Beta is the second Greek letter.',
          'Gamma measures the rate of change of Delta.',
        ].join(' '),
        uri: null,
        permission: 'model-eligible',
        freshness: now,
        createdAt: now,
      });

      const result = await runBetaGeneration(store, {
        runId: 'run-plain',
        sourceDocumentIds: ['src-plain-notes'],
        startedAt: now,
        defaultDue: now - 60_000,
      });

      expect(result.validationFailures).toEqual([]);
      expect(result.acceptedDraftIds).toEqual([
        'run-plain-draft-src-plain-notes-1-alpha',
        'run-plain-draft-src-plain-notes-2-beta',
        'run-plain-draft-src-plain-notes-3-gamma',
        'run-plain-draft-src-plain-notes-4-greek-notes-synthesis',
      ]);
      expect(store.snapshot().referenceSpans.map((span) => span.text)).toEqual([
        'Alpha is the first Greek letter.',
        'Beta is the second Greek letter.',
        'Gamma measures the rate of change of Delta.',
        'Alpha is the first Greek letter. Beta is the second Greek letter.',
      ]);
      expect(store.snapshot().generatedPromptDrafts).toMatchObject([
        {
          prompt: {
            kind: 'mcq',
            prompt: 'According to Greek notes, what is Alpha?',
            correctChoice: 'first Greek letter',
          },
        },
        {
          prompt: {
            kind: 'mcq',
            prompt: 'According to Greek notes, what is Beta?',
            correctChoice: 'second Greek letter',
          },
        },
        {
          prompt: {
            kind: 'mcq',
            prompt: 'According to Greek notes, what is Gamma?',
            correctChoice: 'rate of change of Delta',
          },
        },
        {
          activityKind: 'exercise',
          workedSolution: 'Alpha: first Greek letter. Beta: second Greek letter.',
          scoringRubric:
            'Pass when the answer uses both cited facts without adding unsupported claims.',
          prompt: {
            kind: 'recitation',
            prompt: 'Use the source to explain how Alpha and Beta fit together.',
            acceptedAnswers: ['Alpha: first Greek letter; Beta: second Greek letter'],
          },
        },
      ]);
    });
  });

  test('turns mapping-list input into one quiz per supplied pair plus a generic exercise', async () => {
    await withTempStore(async (path) => {
      const store = await createBetaPersistenceStore(path);
      await store.saveSourceDocument({
        id: 'src-nato-alphabet',
        kind: 'text',
        title: 'NATO phonetic alphabet',
        body: [
          'I want to learn the NATO phonetic alphabet.',
          'A Alfa, B Bravo, C Charlie, D Delta, E Echo, F Foxtrot, G Golf, H Hotel.',
          'I India, J Juliett, K Kilo, L Lima, M Mike, N November, O Oscar.',
          'P Papa, Q Quebec, R Romeo, S Sierra, T Tango, U Uniform.',
          'V Victor, W Whiskey, X X-ray, Y Yankee, Z Zulu.',
        ].join(' '),
        uri: null,
        permission: 'model-eligible',
        freshness: now,
        createdAt: now,
      });

      const result = await runBetaGeneration(store, {
        runId: 'run-nato-alphabet',
        sourceDocumentIds: ['src-nato-alphabet'],
        startedAt: now,
        defaultDue: now - 60_000,
      });

      expect(result.validationFailures).toEqual([]);
      expect(result.acceptedDraftIds).toHaveLength(27);
      expect(result.rejectedDraftIds).toEqual([]);

      const drafts = store.snapshot().generatedPromptDrafts;
      const letterDrafts = drafts.slice(0, 26);
      expect(letterDrafts.map((draft) => draft.queue.conceptKey)).toEqual([
        'a',
        'b',
        'c',
        'd',
        'e',
        'f',
        'g',
        'h',
        'i',
        'j',
        'k',
        'l',
        'm',
        'n',
        'o',
        'p',
        'q',
        'r',
        's',
        't',
        'u',
        'v',
        'w',
        'x',
        'y',
        'z',
      ]);
      expect(letterDrafts.map((draft) => draft.prompt).slice(0, 3)).toMatchObject([
        {
          kind: 'mcq',
          prompt: 'According to NATO phonetic alphabet, what is A?',
          correctChoice: 'Alfa',
        },
        {
          kind: 'mcq',
          prompt: 'According to NATO phonetic alphabet, what is B?',
          correctChoice: 'Bravo',
        },
        {
          kind: 'mcq',
          prompt: 'According to NATO phonetic alphabet, what is C?',
          correctChoice: 'Charlie',
        },
      ]);
      expect(drafts[26]?.prompt).toMatchObject({
        kind: 'recitation',
        prompt: 'Recall the first four mappings from NATO phonetic alphabet.',
        acceptedAnswers: ['A: Alfa; B: Bravo; C: Charlie; D: Delta'],
      });
    });
  });

  test('generates the same mapping ladder for arbitrary non-alphabet input', async () => {
    await withTempStore(async (path) => {
      const store = await createBetaPersistenceStore(path);
      await store.saveSourceDocument({
        id: 'src-color-rules',
        kind: 'text',
        title: 'Color rules',
        body: 'Red: stop, Green: go, Yellow: caution, Blue: information',
        uri: null,
        permission: 'model-eligible',
        freshness: now,
        createdAt: now,
      });

      const result = await runBetaGeneration(store, {
        runId: 'run-color-rules',
        sourceDocumentIds: ['src-color-rules'],
        startedAt: now,
        defaultDue: now - 60_000,
      });

      expect(result.validationFailures).toEqual([]);
      expect(result.acceptedDraftIds).toHaveLength(5);

      const drafts = store.snapshot().generatedPromptDrafts;
      expect(drafts.map((draft) => draft.queue.conceptKey)).toEqual([
        'red',
        'green',
        'yellow',
        'blue',
        'color-rules-mapping-sequence',
      ]);
      expect(drafts.map((draft) => draft.prompt).slice(0, 4)).toMatchObject([
        {
          kind: 'mcq',
          prompt: 'According to Color rules, what is Red?',
          correctChoice: 'stop',
        },
        {
          kind: 'mcq',
          prompt: 'According to Color rules, what is Green?',
          correctChoice: 'go',
        },
        {
          kind: 'mcq',
          prompt: 'According to Color rules, what is Yellow?',
          correctChoice: 'caution',
        },
        {
          kind: 'mcq',
          prompt: 'According to Color rules, what is Blue?',
          correctChoice: 'information',
        },
      ]);
    });
  });

  test('reports arbitrary prose that lacks source-backed facts without saving drafts', async () => {
    await withTempStore(async (path) => {
      const store = await createBetaPersistenceStore(path);
      await store.saveSourceDocument({
        id: 'src-vague',
        kind: 'text',
        title: 'Vague notes',
        body: 'Remember this later maybe somehow.',
        uri: null,
        permission: 'model-eligible',
        freshness: now,
        createdAt: now,
      });

      const result = await runBetaGeneration(store, {
        runId: 'run-vague',
        sourceDocumentIds: ['src-vague'],
        startedAt: now,
        defaultDue: now,
      });

      expect(result).toEqual({
        runId: 'run-vague',
        draftIds: [],
        acceptedDraftIds: [],
        rejectedDraftIds: [],
        validationFailures: [
          'src-vague: arbitrary text did not contain enough source-backed facts to generate practice',
        ],
      });
      expect(store.snapshot().generatedPromptDrafts).toEqual([]);
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
