use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Rating {
    Again = 1,
    Hard = 2,
    Good = 3,
    Easy = 4,
}

impl Serialize for Rating {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u8(*self as u8)
    }
}

impl<'de> Deserialize<'de> for Rating {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match u8::deserialize(deserializer)? {
            1 => Ok(Self::Again),
            2 => Ok(Self::Hard),
            3 => Ok(Self::Good),
            4 => Ok(Self::Easy),
            value => Err(serde::de::Error::custom(format!("invalid rating: {value}"))),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Correct,
    Close,
    Wrong,
    Revealed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum GraderKind {
    #[serde(rename = "deterministic")]
    Deterministic,
    #[serde(rename = "rubric-llm")]
    RubricLlm,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduleStatus {
    New = 0,
    Learning = 1,
    Review = 2,
    Relearning = 3,
}

impl Serialize for ScheduleStatus {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u8(*self as u8)
    }
}

impl<'de> Deserialize<'de> for ScheduleStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match u8::deserialize(deserializer)? {
            0 => Ok(Self::New),
            1 => Ok(Self::Learning),
            2 => Ok(Self::Review),
            3 => Ok(Self::Relearning),
            value => Err(serde::de::Error::custom(format!(
                "invalid schedule status: {value}"
            ))),
        }
    }
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
#[serde(rename_all = "camelCase")]
pub struct ReviewUnitLifecycle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invalidated_at: Option<i64>,
}

impl ReviewUnitLifecycle {
    #[must_use]
    pub const fn active() -> Self {
        Self {
            ttl_expires_at: None,
            invalidated_at: None,
        }
    }

    #[must_use]
    pub const fn ttl_expires_at(ttl_expires_at: i64) -> Self {
        Self {
            ttl_expires_at: Some(ttl_expires_at),
            invalidated_at: None,
        }
    }

    #[must_use]
    pub const fn invalidated_at(invalidated_at: i64) -> Self {
        Self {
            ttl_expires_at: None,
            invalidated_at: Some(invalidated_at),
        }
    }

    #[must_use]
    pub const fn with_ttl_expires_at(mut self, ttl_expires_at: Option<i64>) -> Self {
        self.ttl_expires_at = ttl_expires_at;
        self
    }

    #[must_use]
    pub const fn with_invalidated_at(mut self, invalidated_at: Option<i64>) -> Self {
        self.invalidated_at = invalidated_at;
        self
    }

    #[must_use]
    pub fn retirement_at(&self, now: i64) -> Option<ReviewUnitRetirement> {
        if let Some(invalidated_at) = self.invalidated_at {
            if now >= invalidated_at {
                return Some(ReviewUnitRetirement {
                    reason: ReviewUnitRetirementReason::Invalidated,
                    occurred_at: invalidated_at,
                });
            }
        }

        if let Some(ttl_expires_at) = self.ttl_expires_at {
            if now >= ttl_expires_at {
                return Some(ReviewUnitRetirement {
                    reason: ReviewUnitRetirementReason::TtlExpired,
                    occurred_at: ttl_expires_at,
                });
            }
        }

        None
    }

    #[must_use]
    pub fn is_schedulable(&self, now: i64) -> bool {
        self.retirement_at(now).is_none()
    }
}

impl Default for ReviewUnitLifecycle {
    fn default() -> Self {
        Self::active()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewUnitRetirement {
    pub reason: ReviewUnitRetirementReason,
    pub occurred_at: i64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReviewUnitRetirementReason {
    TtlExpired,
    Invalidated,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GradeContext {
    pub response_time_ms: u32,
    pub prior_reps: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
pub struct RubricCriterionResult {
    pub name: String,
    pub verdict: RubricCriterionVerdict,
    pub evidence: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RubricCriterionVerdict {
    Pass,
    Fail,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RubricCriterion {
    pub name: String,
    pub description: String,
    pub required: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RubricDefinition {
    pub answer_guide: Vec<String>,
    pub passing_score: usize,
    pub criteria: Vec<RubricCriterion>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RubricPrompt {
    pub review_unit_id: ReviewUnitId,
    pub prompt: String,
    pub rubric: RubricDefinition,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RubricAssessment {
    pub model: Option<String>,
    pub confidence: f64,
    pub feedback: String,
    pub criterion_results: Vec<RubricCriterionResult>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
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

impl Serialize for Prompt {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        PromptWire::from(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Prompt {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(PromptWire::deserialize(deserializer)?.into())
    }
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind")]
enum PromptWire {
    #[serde(rename = "mcq", rename_all = "camelCase")]
    Mcq {
        review_unit_id: ReviewUnitId,
        prompt: String,
        choices: Vec<String>,
        correct_choice: String,
    },
    #[serde(rename = "boolean", rename_all = "camelCase")]
    Boolean {
        review_unit_id: ReviewUnitId,
        prompt: String,
        correct_answer: bool,
    },
    #[serde(rename = "cloze", rename_all = "camelCase")]
    Cloze {
        review_unit_id: ReviewUnitId,
        prompt: String,
        accepted_answers: Vec<String>,
        equivalence_groups: Vec<Vec<String>>,
        ignored_tokens: Vec<String>,
    },
    #[serde(rename = "shortAnswer", rename_all = "camelCase")]
    ShortAnswer {
        review_unit_id: ReviewUnitId,
        prompt: String,
        accepted_answers: Vec<String>,
        equivalence_groups: Vec<Vec<String>>,
        ignored_tokens: Vec<String>,
    },
    #[serde(rename = "recitation", rename_all = "camelCase")]
    Recitation {
        review_unit_id: ReviewUnitId,
        prompt: String,
        accepted_answers: Vec<String>,
        equivalence_groups: Vec<Vec<String>>,
        ignored_tokens: Vec<String>,
    },
}

impl From<&Prompt> for PromptWire {
    fn from(prompt: &Prompt) -> Self {
        match prompt {
            Prompt::Mcq {
                review_unit_id,
                prompt,
                choices,
                correct_choice,
            } => Self::Mcq {
                review_unit_id: review_unit_id.clone(),
                prompt: prompt.clone(),
                choices: choices.clone(),
                correct_choice: correct_choice.clone(),
            },
            Prompt::Boolean {
                review_unit_id,
                prompt,
                correct_answer,
            } => Self::Boolean {
                review_unit_id: review_unit_id.clone(),
                prompt: prompt.clone(),
                correct_answer: *correct_answer,
            },
            Prompt::Exact(prompt) => match prompt.kind {
                ExactPromptKind::Cloze => Self::Cloze {
                    review_unit_id: prompt.review_unit_id.clone(),
                    prompt: prompt.prompt.clone(),
                    accepted_answers: prompt.accepted_answers.clone(),
                    equivalence_groups: prompt.equivalence_groups.clone(),
                    ignored_tokens: prompt.ignored_tokens.clone(),
                },
                ExactPromptKind::ShortAnswer => Self::ShortAnswer {
                    review_unit_id: prompt.review_unit_id.clone(),
                    prompt: prompt.prompt.clone(),
                    accepted_answers: prompt.accepted_answers.clone(),
                    equivalence_groups: prompt.equivalence_groups.clone(),
                    ignored_tokens: prompt.ignored_tokens.clone(),
                },
                ExactPromptKind::Recitation => Self::Recitation {
                    review_unit_id: prompt.review_unit_id.clone(),
                    prompt: prompt.prompt.clone(),
                    accepted_answers: prompt.accepted_answers.clone(),
                    equivalence_groups: prompt.equivalence_groups.clone(),
                    ignored_tokens: prompt.ignored_tokens.clone(),
                },
            },
        }
    }
}

impl From<PromptWire> for Prompt {
    fn from(prompt: PromptWire) -> Self {
        match prompt {
            PromptWire::Mcq {
                review_unit_id,
                prompt,
                choices,
                correct_choice,
            } => Self::Mcq {
                review_unit_id,
                prompt,
                choices,
                correct_choice,
            },
            PromptWire::Boolean {
                review_unit_id,
                prompt,
                correct_answer,
            } => Self::Boolean {
                review_unit_id,
                prompt,
                correct_answer,
            },
            PromptWire::Cloze {
                review_unit_id,
                prompt,
                accepted_answers,
                equivalence_groups,
                ignored_tokens,
            } => Self::Exact(ExactPrompt {
                kind: ExactPromptKind::Cloze,
                review_unit_id,
                prompt,
                accepted_answers,
                equivalence_groups,
                ignored_tokens,
            }),
            PromptWire::ShortAnswer {
                review_unit_id,
                prompt,
                accepted_answers,
                equivalence_groups,
                ignored_tokens,
            } => Self::Exact(ExactPrompt {
                kind: ExactPromptKind::ShortAnswer,
                review_unit_id,
                prompt,
                accepted_answers,
                equivalence_groups,
                ignored_tokens,
            }),
            PromptWire::Recitation {
                review_unit_id,
                prompt,
                accepted_answers,
                equivalence_groups,
                ignored_tokens,
            } => Self::Exact(ExactPrompt {
                kind: ExactPromptKind::Recitation,
                review_unit_id,
                prompt,
                accepted_answers,
                equivalence_groups,
                ignored_tokens,
            }),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
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
    #[serde(rename = "cloze")]
    Cloze,
    #[serde(rename = "shortAnswer")]
    ShortAnswer,
    #[serde(rename = "recitation")]
    Recitation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
pub struct QueueCandidate {
    pub review_unit_id: ReviewUnitId,
    pub schedule_state: Option<ScheduleState>,
    pub due: i64,
    #[serde(default)]
    pub lifecycle: ReviewUnitLifecycle,
    pub progression: Option<ProgressionMetadata>,
    pub concept_key: Option<String>,
    pub source_key: Option<String>,
    pub domain_key: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
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
