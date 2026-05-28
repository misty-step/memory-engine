use std::collections::{BTreeMap, BTreeSet};

use crate::{ProgressionMetadata, ReviewUnitId};

pub trait ProgressionLike<TReview> {
    fn review_unit_id(&self) -> &ReviewUnitId;
    fn review(&self) -> Option<&TReview>;
    fn progression(&self) -> Option<&ProgressionMetadata>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgressionCandidate<TReview> {
    pub review_unit_id: ReviewUnitId,
    pub review: Option<TReview>,
    pub progression: Option<ProgressionMetadata>,
}

impl<TReview> ProgressionLike<TReview> for ProgressionCandidate<TReview> {
    fn review_unit_id(&self) -> &ReviewUnitId {
        &self.review_unit_id
    }

    fn review(&self) -> Option<&TReview> {
        self.review.as_ref()
    }

    fn progression(&self) -> Option<&ProgressionMetadata> {
        self.progression.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgressionFilterResult<TCandidate> {
    pub available: Vec<TCandidate>,
    pub locked_fresh_count: usize,
}

#[must_use]
pub fn is_mastered<TReview>(
    review: Option<&TReview>,
    mastery_policy: impl Fn(&TReview) -> bool,
) -> bool {
    review.is_some_and(mastery_policy)
}

#[must_use]
pub fn filter_eligible_candidates<TReview, TCandidate>(
    candidates: &[TCandidate],
    mastery_policy: impl Fn(&TReview) -> bool + Copy,
    population: Option<&[TCandidate]>,
) -> ProgressionFilterResult<TCandidate>
where
    TCandidate: Clone + ProgressionLike<TReview>,
{
    evaluate_strict_candidates(candidates, mastery_policy, population).1
}

#[must_use]
pub fn filter_eligible_candidates_with_fallback<TReview, TCandidate>(
    candidates: &[TCandidate],
    mastery_policy: impl Fn(&TReview) -> bool + Copy,
    population: Option<&[TCandidate]>,
) -> ProgressionFilterResult<TCandidate>
where
    TCandidate: Clone + ProgressionLike<TReview>,
{
    let (context, strict) = evaluate_strict_candidates(candidates, mastery_policy, population);
    if !strict.available.is_empty() {
        return strict;
    }

    ProgressionFilterResult {
        available: candidates
            .iter()
            .filter(|candidate| !context.buried_ids.contains(candidate.review_unit_id()))
            .cloned()
            .collect(),
        locked_fresh_count: strict.locked_fresh_count,
    }
}

#[derive(Clone, Debug, Default)]
struct ProgressionContext {
    buried_ids: BTreeSet<ReviewUnitId>,
    mastered_ids: BTreeSet<ReviewUnitId>,
    known_stages_by_group: BTreeMap<String, BTreeSet<u32>>,
    mastered_stages_by_group: BTreeMap<String, BTreeSet<u32>>,
}

fn evaluate_strict_candidates<TReview, TCandidate>(
    candidates: &[TCandidate],
    mastery_policy: impl Fn(&TReview) -> bool + Copy,
    population: Option<&[TCandidate]>,
) -> (ProgressionContext, ProgressionFilterResult<TCandidate>)
where
    TCandidate: Clone + ProgressionLike<TReview>,
{
    let context = build_context(population.unwrap_or(candidates), mastery_policy);
    let available = candidates
        .iter()
        .filter(|candidate| is_eligible(*candidate, &context))
        .cloned()
        .collect::<Vec<_>>();
    let locked_fresh_count = count_locked_fresh_candidates(candidates, &available);

    (
        context,
        ProgressionFilterResult {
            available,
            locked_fresh_count,
        },
    )
}

fn build_context<TReview, TCandidate>(
    population: &[TCandidate],
    mastery_policy: impl Fn(&TReview) -> bool,
) -> ProgressionContext
where
    TCandidate: ProgressionLike<TReview>,
{
    let mut context = ProgressionContext::default();

    for candidate in population {
        let progression = ProgressionMetadata::normalized(candidate.progression());

        if let Some(group) = progression.progression_group.as_ref() {
            context
                .known_stages_by_group
                .entry(group.clone())
                .or_default()
                .insert(progression.stage_order);
        }

        if !is_mastered(candidate.review(), &mastery_policy) {
            continue;
        }

        context
            .mastered_ids
            .insert(candidate.review_unit_id().clone());

        for review_unit_id in progression.supersedes {
            context.buried_ids.insert(review_unit_id);
        }

        if let Some(group) = progression.progression_group {
            context
                .mastered_stages_by_group
                .entry(group)
                .or_default()
                .insert(progression.stage_order);
        }
    }

    context
}

fn is_eligible<TReview>(
    candidate: &impl ProgressionLike<TReview>,
    context: &ProgressionContext,
) -> bool {
    let progression = ProgressionMetadata::normalized(candidate.progression());
    if context.buried_ids.contains(candidate.review_unit_id()) {
        return false;
    }

    if !progression
        .requires
        .iter()
        .all(|review_unit_id| context.mastered_ids.contains(review_unit_id))
    {
        return false;
    }

    if candidate.review().is_some() {
        return true;
    }

    let Some(group) = progression.progression_group else {
        return true;
    };
    if progression.stage_order <= 1 {
        return true;
    }

    let known_stages = context.known_stages_by_group.get(&group);
    let mastered_stages = context.mastered_stages_by_group.get(&group);

    known_stages.is_none_or(|stages| {
        stages
            .iter()
            .filter(|stage| **stage < progression.stage_order)
            .all(|stage| mastered_stages.is_some_and(|mastered| mastered.contains(stage)))
    })
}

fn count_locked_fresh_candidates<TReview, TCandidate>(
    candidates: &[TCandidate],
    available: &[TCandidate],
) -> usize
where
    TCandidate: ProgressionLike<TReview>,
{
    let available_fresh_count = available
        .iter()
        .filter(|candidate| candidate.review().is_none())
        .count();
    let total_fresh_count = candidates
        .iter()
        .filter(|candidate| candidate.review().is_none())
        .count();

    total_fresh_count.saturating_sub(available_fresh_count)
}

#[cfg(test)]
mod tests {
    use crate::{ProgressionMetadata, ReviewUnitId, ScheduleState, ScheduleStatus};

    use super::*;

    fn state(status: ScheduleStatus, reps: u32) -> ScheduleState {
        ScheduleState {
            due: 0,
            stability: 0.0,
            difficulty: 0.0,
            elapsed_days: 0,
            scheduled_days: 0,
            reps,
            lapses: 0,
            state: status,
            last_review: None,
        }
    }

    fn candidate(
        id: &str,
        review: Option<ScheduleState>,
        progression: Option<ProgressionMetadata>,
    ) -> ProgressionCandidate<ScheduleState> {
        ProgressionCandidate {
            review_unit_id: ReviewUnitId::new(id),
            review,
            progression,
        }
    }

    fn progression(group: &str, stage_order: u32) -> ProgressionMetadata {
        ProgressionMetadata {
            progression_group: Some(group.to_owned()),
            stage_order,
            requires: Vec::new(),
            supersedes: Vec::new(),
        }
    }

    fn mastered(review: &ScheduleState) -> bool {
        review.state == ScheduleStatus::Review && review.reps >= 3
    }

    #[test]
    fn keeps_later_fresh_stages_locked_until_prior_stages_are_mastered() {
        let candidates = vec![
            candidate("a-stage-1", None, Some(progression("Concept-A", 1))),
            candidate("a-stage-2", None, Some(progression(" concept-a ", 2))),
        ];

        let result = filter_eligible_candidates(&candidates, mastered, None);

        assert_eq!(
            result
                .available
                .iter()
                .map(|entry| entry.review_unit_id.as_str())
                .collect::<Vec<_>>(),
            vec!["a-stage-1"]
        );
        assert_eq!(result.locked_fresh_count, 1);
    }

    #[test]
    fn unlocks_next_stage_from_wider_population() {
        let stage_one = candidate(
            "a-stage-1",
            Some(state(ScheduleStatus::Review, 3)),
            Some(progression("concept-a", 1)),
        );
        let stage_two = candidate("a-stage-2", None, Some(progression("concept-a", 2)));

        let result = filter_eligible_candidates(
            std::slice::from_ref(&stage_two),
            mastered,
            Some(&[stage_one, stage_two.clone()]),
        );

        assert_eq!(result.available, vec![stage_two]);
        assert_eq!(result.locked_fresh_count, 0);
    }

    #[test]
    fn suppresses_superseded_units_and_falls_back_when_everything_is_locked() {
        let easier = candidate("easy", None, Some(progression("prayer", 1)));
        let mut harder_progression = progression("prayer", 3);
        harder_progression.supersedes = vec![ReviewUnitId::new("easy")];
        let harder = candidate(
            "hard",
            Some(state(ScheduleStatus::Review, 4)),
            Some(harder_progression),
        );

        let strict = filter_eligible_candidates(
            std::slice::from_ref(&easier),
            mastered,
            Some(&[easier.clone(), harder]),
        );
        assert!(strict.available.is_empty());

        let locked = candidate("locked", None, Some(progression("other", 2)));
        let fallback =
            filter_eligible_candidates_with_fallback(std::slice::from_ref(&locked), mastered, None);
        assert_eq!(fallback.available, vec![locked]);
    }
}
