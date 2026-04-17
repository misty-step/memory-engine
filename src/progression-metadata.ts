import type { ProgressionMetadata, ReviewUnitId } from './types';

export function normalizeProgressionMetadata(
  progression: ProgressionMetadata | null,
): ProgressionMetadata {
  if (progression === null) {
    return {
      progressionGroup: null,
      stageOrder: 1,
      requires: [],
      supersedes: [],
    };
  }

  return {
    progressionGroup: normalizeProgressionGroup(progression.progressionGroup),
    stageOrder: normalizeStageOrder(progression.stageOrder),
    requires: dedupeIds(progression.requires),
    supersedes: dedupeIds(progression.supersedes),
  };
}

function normalizeProgressionGroup(value: string | null): string | null {
  const normalized = value?.trim().toLowerCase();

  return normalized ? normalized : null;
}

function normalizeStageOrder(value: number): number {
  if (!Number.isFinite(value)) {
    return 1;
  }

  return Math.max(1, Math.trunc(value));
}

function dedupeIds(values: readonly ReviewUnitId[]): ReviewUnitId[] {
  return Array.from(new Set(values));
}
