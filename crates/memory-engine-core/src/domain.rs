use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ReviewUnitId(String);

impl ReviewUnitId {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ReviewUnitId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Rating {
    Again = 1,
    Hard = 2,
    Good = 3,
    Easy = 4,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Verdict {
    Correct,
    Close,
    Wrong,
    Revealed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum GraderKind {
    Deterministic,
    RubricLlm,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ScheduleStatus {
    New = 0,
    Learning = 1,
    Review = 2,
    Relearning = 3,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ScheduleState {
    pub due: i64,
    pub stability: f64,
    pub difficulty: f64,
    pub elapsed_days: i64,
    pub scheduled_days: i64,
    pub reps: u32,
    pub lapses: u32,
    pub state: ScheduleStatus,
    pub last_review: Option<i64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GradeContext {
    pub response_time_ms: u32,
    pub prior_reps: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GradeResult {
    pub verdict: Verdict,
    pub rating: Rating,
    pub is_correct: bool,
    pub submitted_answer: String,
    pub expected_answer: String,
    pub grader_kind: GraderKind,
    pub grader_model: Option<String>,
    pub grader_confidence: Option<f64>,
    pub feedback: String,
    pub criterion_results: Vec<RubricCriterionResult>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RubricCriterionResult {
    pub name: String,
    pub verdict: RubricCriterionVerdict,
    pub evidence: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum RubricCriterionVerdict {
    Pass,
    Fail,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Prompt {
    Mcq {
        review_unit_id: ReviewUnitId,
        prompt: String,
        choices: Vec<String>,
        correct_choice: String,
    },
    Boolean {
        review_unit_id: ReviewUnitId,
        prompt: String,
        correct_answer: bool,
    },
    Exact(ExactPrompt),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExactPrompt {
    pub kind: ExactPromptKind,
    pub review_unit_id: ReviewUnitId,
    pub prompt: String,
    pub accepted_answers: Vec<String>,
    pub equivalence_groups: Vec<Vec<String>>,
    pub ignored_tokens: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ExactPromptKind {
    Cloze,
    ShortAnswer,
    Recitation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProgressionMetadata {
    pub progression_group: Option<String>,
    pub stage_order: u32,
    pub requires: Vec<ReviewUnitId>,
    pub supersedes: Vec<ReviewUnitId>,
}

impl ProgressionMetadata {
    #[must_use]
    pub fn normalized(value: Option<&Self>) -> Self {
        let Some(value) = value else {
            return Self::default();
        };

        Self {
            progression_group: normalize_group(value.progression_group.as_deref()),
            stage_order: value.stage_order.max(1),
            requires: dedupe_ids(&value.requires),
            supersedes: dedupe_ids(&value.supersedes),
        }
    }
}

impl Default for ProgressionMetadata {
    fn default() -> Self {
        Self {
            progression_group: None,
            stage_order: 1,
            requires: Vec::new(),
            supersedes: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct QueueCandidate {
    pub review_unit_id: ReviewUnitId,
    pub schedule_state: Option<ScheduleState>,
    pub due: i64,
    pub progression: Option<ProgressionMetadata>,
    pub concept_key: Option<String>,
    pub source_key: Option<String>,
    pub domain_key: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueueSeparationPass {
    pub concept: bool,
    pub source: bool,
    pub domain: bool,
}

#[derive(Clone, Debug)]
pub struct QueueSelectionOptions<'a> {
    pub now: i64,
    pub recent_candidates: &'a [QueueCandidate],
    pub population: Option<&'a [QueueCandidate]>,
    pub candidate_window: usize,
    pub recent_concept_window: usize,
    pub recent_source_window: usize,
    pub recent_domain_window: usize,
    pub separation_passes: &'a [QueueSeparationPass],
}

impl Default for QueueSelectionOptions<'_> {
    fn default() -> Self {
        Self {
            now: 0,
            recent_candidates: &[],
            population: None,
            candidate_window: 12,
            recent_concept_window: 3,
            recent_source_window: 2,
            recent_domain_window: 1,
            separation_passes: &[
                QueueSeparationPass {
                    concept: true,
                    source: true,
                    domain: true,
                },
                QueueSeparationPass {
                    concept: true,
                    source: true,
                    domain: false,
                },
                QueueSeparationPass {
                    concept: true,
                    source: false,
                    domain: false,
                },
                QueueSeparationPass {
                    concept: false,
                    source: true,
                    domain: false,
                },
                QueueSeparationPass {
                    concept: false,
                    source: false,
                    domain: false,
                },
            ],
        }
    }
}

fn normalize_group(value: Option<&str>) -> Option<String> {
    let normalized = value?.trim().to_lowercase();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn dedupe_ids(values: &[ReviewUnitId]) -> Vec<ReviewUnitId> {
    let mut deduped = Vec::new();
    for value in values {
        if !deduped.contains(value) {
            deduped.push(value.clone());
        }
    }
    deduped
}
