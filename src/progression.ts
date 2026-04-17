import { normalizeProgressionMetadata } from './progression-metadata';
import type {
  MasteryPolicy,
  ProgressionCandidate,
  ProgressionFilterResult,
  ReviewUnitId,
} from './types';

type FilterOptions<TCandidate> = {
  population?: readonly TCandidate[];
};

type ProgressionContext = {
  buriedIds: Set<ReviewUnitId>;
  masteredIds: Set<ReviewUnitId>;
  knownStagesByGroup: Map<string, number[]>;
  masteredStagesByGroup: Map<string, Set<number>>;
};

export function isMastered<TReview>(
  review: TReview | null,
  masteryPolicy: MasteryPolicy<TReview>,
): boolean {
  return review !== null && masteryPolicy(review);
}

export function filterEligibleCandidates<TReview, TCandidate extends ProgressionCandidate<TReview>>(
  candidates: readonly TCandidate[],
  masteryPolicy: MasteryPolicy<TReview>,
  options: FilterOptions<TCandidate> = {},
): ProgressionFilterResult<TCandidate> {
  return evaluateStrictCandidates(candidates, masteryPolicy, options).result;
}

export function filterEligibleCandidatesWithFallback<
  TReview,
  TCandidate extends ProgressionCandidate<TReview>,
>(
  candidates: readonly TCandidate[],
  masteryPolicy: MasteryPolicy<TReview>,
  options: FilterOptions<TCandidate> = {},
): ProgressionFilterResult<TCandidate> {
  const { context, result: strict } = evaluateStrictCandidates(candidates, masteryPolicy, options);
  if (strict.available.length > 0) {
    return strict;
  }

  return {
    available: candidates.filter((candidate) => !context.buriedIds.has(candidate.reviewUnitId)),
    lockedFreshCount: strict.lockedFreshCount,
  };
}

function evaluateStrictCandidates<TReview, TCandidate extends ProgressionCandidate<TReview>>(
  candidates: readonly TCandidate[],
  masteryPolicy: MasteryPolicy<TReview>,
  options: FilterOptions<TCandidate>,
): {
  context: ProgressionContext;
  result: ProgressionFilterResult<TCandidate>;
} {
  const population = options.population ?? candidates;
  const context = buildContext(population, masteryPolicy);
  const available = candidates.filter((candidate) => isEligible(candidate, context));

  return {
    context,
    result: {
      available,
      lockedFreshCount: countLockedFreshCandidates(candidates, available),
    },
  };
}

function buildContext<TReview, TCandidate extends ProgressionCandidate<TReview>>(
  population: readonly TCandidate[],
  masteryPolicy: MasteryPolicy<TReview>,
): ProgressionContext {
  const buriedIds = new Set<ReviewUnitId>();
  const masteredIds = new Set<ReviewUnitId>();
  const knownStagesByGroup = new Map<string, number[]>();
  const masteredStagesByGroup = new Map<string, Set<number>>();

  for (const candidate of population) {
    const progression = normalizeProgressionMetadata(candidate.progression);

    if (progression.progressionGroup !== null) {
      const knownStages = knownStagesByGroup.get(progression.progressionGroup) ?? [];
      if (!knownStages.includes(progression.stageOrder)) {
        knownStages.push(progression.stageOrder);
        knownStages.sort((left, right) => left - right);
        knownStagesByGroup.set(progression.progressionGroup, knownStages);
      }
    }

    if (!isMastered(candidate.review, masteryPolicy)) {
      continue;
    }

    masteredIds.add(candidate.reviewUnitId);

    for (const reviewUnitId of progression.supersedes) {
      buriedIds.add(reviewUnitId);
    }

    if (progression.progressionGroup !== null) {
      const masteredStages =
        masteredStagesByGroup.get(progression.progressionGroup) ?? new Set<number>();
      masteredStages.add(progression.stageOrder);
      masteredStagesByGroup.set(progression.progressionGroup, masteredStages);
    }
  }

  return {
    buriedIds,
    masteredIds,
    knownStagesByGroup,
    masteredStagesByGroup,
  };
}

function isEligible<TReview>(
  candidate: ProgressionCandidate<TReview>,
  context: ProgressionContext,
): boolean {
  const progression = normalizeProgressionMetadata(candidate.progression);
  if (context.buriedIds.has(candidate.reviewUnitId)) {
    return false;
  }

  if (!progression.requires.every((reviewUnitId) => context.masteredIds.has(reviewUnitId))) {
    return false;
  }

  if (candidate.review !== null) {
    return true;
  }

  if (progression.progressionGroup === null || progression.stageOrder <= 1) {
    return true;
  }

  const knownStages = context.knownStagesByGroup.get(progression.progressionGroup) ?? [];
  const masteredStages =
    context.masteredStagesByGroup.get(progression.progressionGroup) ?? new Set<number>();

  return knownStages
    .filter((stageOrder) => stageOrder < progression.stageOrder)
    .every((stageOrder) => masteredStages.has(stageOrder));
}

function countLockedFreshCandidates<TReview, TCandidate extends ProgressionCandidate<TReview>>(
  candidates: readonly TCandidate[],
  available: readonly TCandidate[],
): number {
  const availableFreshCount = available.filter((candidate) => candidate.review === null).length;
  const totalFreshCount = candidates.filter((candidate) => candidate.review === null).length;

  return Math.max(0, totalFreshCount - availableFreshCount);
}
