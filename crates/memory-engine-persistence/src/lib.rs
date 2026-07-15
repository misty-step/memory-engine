//! File-backed beta persistence store for memory-engine.
//!
//! This crate owns durable beta-study state and implements the service store
//! trait. It intentionally keeps filesystem details here, outside the pure core
//! and service orchestration crates.

use std::{
    collections::BTreeSet,
    error::Error,
    fmt, fs,
    fs::OpenOptions,
    io,
    path::{Path, PathBuf},
};

use memory_engine_core::{
    defer_queue_availability, Prompt, QueueCandidate, ReviewUnitId, ReviewUnitLifecycle,
    ScheduleState,
};
use memory_engine_service::{
    content_feedback_replay_matches, ContentFeedback, ContentFeedbackStore, ContentFeedbackVerdict,
    MemoryServiceStore, ServiceAttemptRecord,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum SourceDocumentKind {
    #[serde(rename = "text")]
    Text,
    #[serde(rename = "link")]
    Link,
    #[serde(rename = "file")]
    File,
    #[serde(rename = "image")]
    Image,
    #[serde(rename = "video-transcript")]
    VideoTranscript,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum SourcePermission {
    #[serde(rename = "local-only")]
    LocalOnly,
    #[serde(rename = "model-eligible")]
    ModelEligible,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDocument {
    pub id: String,
    pub kind: SourceDocumentKind,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_key: Option<String>,
    pub body: Option<String>,
    pub uri: Option<String>,
    pub permission: SourcePermission,
    pub freshness: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_expires_at: Option<i64>,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceSpan {
    pub id: String,
    pub source_document_id: String,
    pub label: String,
    pub text: String,
    pub locator: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum GeneratedPromptValidationStatus {
    #[serde(rename = "pending")]
    Pending,
    #[serde(rename = "accepted")]
    Accepted,
    #[serde(rename = "rejected")]
    Rejected,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedPromptValidation {
    pub status: GeneratedPromptValidationStatus,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedPromptModel {
    pub provider: String,
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum GeneratedLearningActivityKind {
    #[serde(rename = "quiz")]
    Quiz,
    #[serde(rename = "exercise")]
    Exercise,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedPromptDraft {
    pub id: String,
    pub source_document_ids: Vec<String>,
    pub reference_span_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concept_reference_note_key: Option<String>,
    pub generation_run_id: Option<String>,
    pub review_unit_id: ReviewUnitId,
    pub prompt_id: String,
    pub prompt: Prompt,
    pub queue: PersistedQueueCandidate,
    pub activity_kind: GeneratedLearningActivityKind,
    pub activity_stage: String,
    pub worked_solution: Option<String>,
    pub model: GeneratedPromptModel,
    pub validation: GeneratedPromptValidation,
    pub critique_notes: Vec<String>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationRun {
    pub id: String,
    pub source_document_ids: Vec<String>,
    #[serde(default)]
    pub parent_review_unit_id: Option<ReviewUnitId>,
    pub draft_ids: Vec<String>,
    pub provider: String,
    pub model: String,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub validation_failures: Vec<String>,
    #[serde(default)]
    pub usage: Option<GenerationRunUsage>,
}

/// Token and cost accounting for one generation run, summed across sources.
///
/// Cost is stored in integer micro-USD so run records stay `Eq` and JSON-safe;
/// `None` means the provider did not report a cost (for example deterministic
/// local providers).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationRunUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd_micros: Option<i64>,
    pub latency_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConceptReferenceNote {
    pub concept_key: String,
    pub title: String,
    pub body: String,
    pub model: GeneratedPromptModel,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BetaReviewUnitRecord {
    pub review_unit_id: ReviewUnitId,
    pub prompt_id: String,
    pub prompt: Prompt,
    pub queue: PersistedQueueCandidate,
    pub reference_span_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concept_reference_note_key: Option<String>,
    pub generated_prompt_draft_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snoozed_until: Option<i64>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedQueueCandidate {
    pub review_unit_id: ReviewUnitId,
    pub due: i64,
    #[serde(default)]
    pub lifecycle: ReviewUnitLifecycle,
    pub progression: Option<memory_engine_core::ProgressionMetadata>,
    pub concept_key: Option<String>,
    pub source_key: Option<String>,
    pub domain_key: Option<String>,
}

impl PersistedQueueCandidate {
    #[must_use]
    pub fn from_queue_candidate(candidate: &QueueCandidate) -> Self {
        Self {
            review_unit_id: candidate.review_unit_id.clone(),
            due: candidate.due,
            lifecycle: candidate.lifecycle,
            progression: candidate.progression.clone(),
            concept_key: candidate.concept_key.clone(),
            source_key: candidate.source_key.clone(),
            domain_key: candidate.domain_key.clone(),
        }
    }

    #[must_use]
    pub fn with_schedule(&self, schedule_state: Option<ScheduleState>) -> QueueCandidate {
        QueueCandidate {
            review_unit_id: self.review_unit_id.clone(),
            due: schedule_state.as_ref().map_or(self.due, |state| state.due),
            schedule_state,
            lifecycle: self.lifecycle,
            progression: self.progression.clone(),
            concept_key: self.concept_key.clone(),
            source_key: self.source_key.clone(),
            domain_key: self.domain_key.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleRecord {
    pub review_unit_id: ReviewUnitId,
    pub state: ScheduleState,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedReviewReceipt {
    pub key: String,
    pub attempt: ServiceAttemptRecord,
    pub expected_prior_schedule_state: Option<ScheduleState>,
    pub schedule_state: ScheduleState,
}

/// A resolved feedback row for the bench calibration label contract.
///
/// The generation configuration fields are deliberately export-only. The
/// append-only [`ContentFeedback`] row stores only the review-unit join key;
/// provenance is resolved through draft and run records at export time.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ContentFeedbackExport {
    #[serde(alias = "feedbackId")]
    pub feedback_id: String,
    #[serde(alias = "reviewUnitId")]
    pub review_unit_id: ReviewUnitId,
    #[serde(alias = "judgeKeep")]
    pub judge_keep: bool,
    #[serde(alias = "humanKeep")]
    pub human_keep: bool,
    pub question: String,
    pub rationale: Option<String>,
    #[serde(rename = "gen_ai.system")]
    pub gen_ai_system: String,
    #[serde(rename = "gen_ai.request.model")]
    pub gen_ai_request_model: String,
    #[serde(rename = "gen_ai.prompt.version")]
    pub gen_ai_prompt_version: String,
    #[serde(rename = "gen_ai.evaluation.score.value")]
    pub gen_ai_evaluation_score_value: f64,
    #[serde(rename = "gen_ai.evaluation.explanation")]
    pub gen_ai_evaluation_explanation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixture: Option<ContentFeedbackFixture>,
}

/// A dropped-content row in the generation bench's corpus shape.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ContentFeedbackFixture {
    pub id: String,
    pub title: String,
    pub category: String,
    pub body: String,
    pub expect: ContentFeedbackFixtureExpect,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContentFeedbackFixtureExpect {
    pub min_drafts: usize,
    pub max_drafts: usize,
    pub key_terms: Vec<String>,
    pub intent: String,
    pub required_activity_kinds: Vec<String>,
    pub required_activity_stage_terms: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BetaStoreSnapshot {
    pub version: u32,
    pub source_documents: Vec<SourceDocument>,
    pub reference_spans: Vec<ReferenceSpan>,
    pub generated_prompt_drafts: Vec<GeneratedPromptDraft>,
    pub review_units: Vec<BetaReviewUnitRecord>,
    pub schedules: Vec<ScheduleRecord>,
    pub attempts: Vec<ServiceAttemptRecord>,
    pub generation_runs: Vec<GenerationRun>,
    #[serde(default)]
    pub content_feedback: Vec<ContentFeedback>,
    pub applied_reviews: Vec<AppliedReviewReceipt>,
    #[serde(default)]
    pub concept_reference_notes: Vec<ConceptReferenceNote>,
}

impl Default for BetaStoreSnapshot {
    fn default() -> Self {
        Self {
            version: 1,
            source_documents: Vec::new(),
            reference_spans: Vec::new(),
            generated_prompt_drafts: Vec::new(),
            review_units: Vec::new(),
            schedules: Vec::new(),
            attempts: Vec::new(),
            generation_runs: Vec::new(),
            content_feedback: Vec::new(),
            applied_reviews: Vec::new(),
            concept_reference_notes: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ApproveGeneratedPromptDraftOptions {
    pub initial_schedule_state: Option<ScheduleState>,
}

#[derive(Debug)]
pub enum BetaStoreError {
    Io(io::Error),
    Json(serde_json::Error),
    UnsupportedVersion(u32),
    Blank {
        label: &'static str,
    },
    UnknownSourceDocument(String),
    UnknownReferenceSpan(String),
    UnknownConceptReferenceNote(String),
    UnknownReviewUnit(ReviewUnitId),
    UnknownGeneratedPromptDraft(String),
    RejectedGeneratedPromptDraft,
    MissingGenerationRunForAcceptedDraft,
    GeneratedPromptDraftRequiresSource,
    GeneratedPromptDraftRequiresReference,
    GeneratedPromptDraftReviewUnitMismatch,
    ReviewUnitMismatch,
    ReviewUnitArchived(ReviewUnitId),
    AttemptAnswerBlank,
    AttemptResponseTimeNonPositive,
    ScheduleLastReviewMismatch,
    DuplicateAppliedReview(String),
    DuplicateContentFeedback(String),
    FeedbackSupersedesUnknown(String),
    FeedbackSupersedesOtherReviewUnit(String),
    FeedbackSupersedesOtherAccount(String),
    FeedbackSupersedesStale {
        expected_head: Option<String>,
        supplied_parent: Option<String>,
    },
    MissingFeedbackProvenance(ReviewUnitId),
    StaleScheduleWrite(ReviewUnitId),
    InjectedCommitFailure,
}

impl fmt::Display for BetaStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::Json(error) => write!(formatter, "JSON error: {error}"),
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "Unsupported beta store snapshot version: {version}"
                )
            }
            Self::Blank { label } => write!(formatter, "{label} must not be blank"),
            Self::UnknownSourceDocument(id) => write!(formatter, "Unknown source document: {id}"),
            Self::UnknownReferenceSpan(id) => write!(formatter, "Unknown reference span: {id}"),
            Self::UnknownConceptReferenceNote(id) => {
                write!(formatter, "Unknown concept reference note: {id}")
            }
            Self::UnknownReviewUnit(id) => write!(formatter, "Unknown review unit: {id}"),
            Self::UnknownGeneratedPromptDraft(id) => {
                write!(formatter, "Unknown generated prompt draft: {id}")
            }
            Self::RejectedGeneratedPromptDraft => {
                formatter.write_str("Only accepted generated prompt drafts can be approved")
            }
            Self::MissingGenerationRunForAcceptedDraft => formatter.write_str(
                "Accepted generated prompt drafts require a generation run before approval",
            ),
            Self::GeneratedPromptDraftRequiresSource => {
                formatter.write_str("Generated prompt drafts require at least one source document")
            }
            Self::GeneratedPromptDraftRequiresReference => {
                formatter.write_str("Generated prompt drafts require at least one reference span")
            }
            Self::GeneratedPromptDraftReviewUnitMismatch => {
                formatter.write_str("Generated prompt draft review unit ids must match")
            }
            Self::ReviewUnitMismatch => {
                formatter.write_str("Review unit ids must match prompt and queue ids")
            }
            Self::ReviewUnitArchived(id) => write!(formatter, "Review unit is archived: {id}"),
            Self::AttemptAnswerBlank => formatter.write_str("Attempt answer must not be blank"),
            Self::AttemptResponseTimeNonPositive => {
                formatter.write_str("Attempt response time must be a positive integer")
            }
            Self::ScheduleLastReviewMismatch => {
                formatter.write_str("Schedule last_review must match the attempt timestamp")
            }
            Self::DuplicateAppliedReview(key) => {
                write!(formatter, "Duplicate applied review: {key}")
            }
            Self::DuplicateContentFeedback(id) => {
                write!(formatter, "Duplicate content feedback id: {id}")
            }
            Self::FeedbackSupersedesUnknown(id) => {
                write!(formatter, "Content feedback supersedes unknown id: {id}")
            }
            Self::FeedbackSupersedesOtherReviewUnit(id) => write!(
                formatter,
                "Content feedback supersedes a different review unit: {id}"
            ),
            Self::FeedbackSupersedesOtherAccount(id) => write!(
                formatter,
                "Content feedback supersedes another account's feedback: {id}"
            ),
            Self::FeedbackSupersedesStale {
                expected_head,
                supplied_parent,
            } => write!(
                formatter,
                "Content feedback revision is stale: expected head {expected_head:?}, supplied parent {supplied_parent:?}"
            ),
            Self::MissingFeedbackProvenance(id) => {
                write!(formatter, "No generation provenance for review unit: {id}")
            }
            Self::StaleScheduleWrite(id) => {
                write!(formatter, "Stale schedule write for review unit: {id}")
            }
            Self::InjectedCommitFailure => {
                formatter.write_str("Injected beta store commit failure")
            }
        }
    }
}

impl Error for BetaStoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

impl PartialEq for BetaStoreError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Io(left), Self::Io(right)) => left.kind() == right.kind(),
            (Self::Json(left), Self::Json(right)) => left.to_string() == right.to_string(),
            (Self::UnsupportedVersion(left), Self::UnsupportedVersion(right)) => left == right,
            (Self::Blank { label: left }, Self::Blank { label: right }) => left == right,
            (Self::UnknownSourceDocument(left), Self::UnknownSourceDocument(right))
            | (Self::UnknownReferenceSpan(left), Self::UnknownReferenceSpan(right))
            | (Self::UnknownConceptReferenceNote(left), Self::UnknownConceptReferenceNote(right))
            | (Self::UnknownGeneratedPromptDraft(left), Self::UnknownGeneratedPromptDraft(right))
            | (Self::DuplicateAppliedReview(left), Self::DuplicateAppliedReview(right))
            | (Self::DuplicateContentFeedback(left), Self::DuplicateContentFeedback(right))
            | (Self::FeedbackSupersedesUnknown(left), Self::FeedbackSupersedesUnknown(right))
            | (
                Self::FeedbackSupersedesOtherReviewUnit(left),
                Self::FeedbackSupersedesOtherReviewUnit(right),
            )
            | (
                Self::FeedbackSupersedesOtherAccount(left),
                Self::FeedbackSupersedesOtherAccount(right),
            ) => left == right,
            (
                Self::FeedbackSupersedesStale {
                    expected_head: left_expected,
                    supplied_parent: left_supplied,
                },
                Self::FeedbackSupersedesStale {
                    expected_head: right_expected,
                    supplied_parent: right_supplied,
                },
            ) => left_expected == right_expected && left_supplied == right_supplied,
            (Self::UnknownReviewUnit(left), Self::UnknownReviewUnit(right))
            | (Self::ReviewUnitArchived(left), Self::ReviewUnitArchived(right))
            | (Self::StaleScheduleWrite(left), Self::StaleScheduleWrite(right))
            | (Self::MissingFeedbackProvenance(left), Self::MissingFeedbackProvenance(right)) => {
                left == right
            }
            (Self::RejectedGeneratedPromptDraft, Self::RejectedGeneratedPromptDraft)
            | (
                Self::MissingGenerationRunForAcceptedDraft,
                Self::MissingGenerationRunForAcceptedDraft,
            )
            | (
                Self::GeneratedPromptDraftRequiresSource,
                Self::GeneratedPromptDraftRequiresSource,
            )
            | (
                Self::GeneratedPromptDraftRequiresReference,
                Self::GeneratedPromptDraftRequiresReference,
            )
            | (
                Self::GeneratedPromptDraftReviewUnitMismatch,
                Self::GeneratedPromptDraftReviewUnitMismatch,
            )
            | (Self::ReviewUnitMismatch, Self::ReviewUnitMismatch)
            | (Self::AttemptAnswerBlank, Self::AttemptAnswerBlank)
            | (Self::AttemptResponseTimeNonPositive, Self::AttemptResponseTimeNonPositive)
            | (Self::ScheduleLastReviewMismatch, Self::ScheduleLastReviewMismatch)
            | (Self::InjectedCommitFailure, Self::InjectedCommitFailure) => true,
            _ => false,
        }
    }
}

impl Eq for BetaStoreError {}

impl From<io::Error> for BetaStoreError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for BetaStoreError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

pub struct BetaPersistenceStore {
    path: PathBuf,
    data: BetaStoreSnapshot,
    fail_next_commit: bool,
}

impl BetaPersistenceStore {
    /// Open or create a beta persistence store at a JSON file path.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStoreError`] when the existing file cannot be read or
    /// decoded, or when it uses an unsupported snapshot version.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, BetaStoreError> {
        let path = path.into();
        let data = load_snapshot(&path)?;

        Ok(Self {
            path,
            data,
            fail_next_commit: false,
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn snapshot(&self) -> BetaStoreSnapshot {
        load_snapshot(&self.path).unwrap_or_else(|_| self.data.clone())
    }

    /// Copy this account's durable snapshot for a new account scope.
    ///
    /// Content feedback carries its account scope because it is also exported
    /// independently of the account snapshot. Rewrite that field while
    /// preserving every other persisted record and use the same atomic file
    /// ownership boundary as ordinary store writes.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStoreError`] when the source cannot be read or the target
    /// cannot be locked and written.
    pub fn copy_for_account(
        &self,
        target_path: impl Into<PathBuf>,
        target_account_id: &str,
    ) -> Result<(), BetaStoreError> {
        assert_non_blank(target_account_id, "Target account id")?;
        let mut snapshot = self.snapshot();
        for feedback in &mut snapshot.content_feedback {
            target_account_id.clone_into(&mut feedback.account_id);
        }

        let target_path = target_path.into();
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let _lock = StoreFileLock::acquire(&target_path)?;
        persist_snapshot(&target_path, &snapshot)
    }

    pub fn fail_next_commit_for_test(&mut self) {
        self.fail_next_commit = true;
    }

    /// Save or replace source material.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStoreError`] for validation or commit failures.
    pub fn save_source_document(
        &mut self,
        document: SourceDocument,
    ) -> Result<SourceDocument, BetaStoreError> {
        assert_non_blank(&document.id, "Source document id")?;
        assert_non_blank(&document.title, "Source document title")?;
        let mut next = self.data.clone();
        upsert_by_id(&mut next.source_documents, document.clone());
        self.commit(&next)?;

        Ok(document)
    }

    /// Hide source material from learner-facing flows while preserving receipts.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStoreError`] when the source document is unknown or cannot
    /// be persisted.
    pub fn archive_source_document(
        &mut self,
        source_document_id: &str,
        archived_at: i64,
    ) -> Result<SourceDocument, BetaStoreError> {
        let mut next = self.data.clone();
        let source = next
            .source_documents
            .iter_mut()
            .find(|source| source.id == source_document_id)
            .ok_or_else(|| BetaStoreError::UnknownSourceDocument(source_document_id.to_owned()))?;
        source.archived_at = Some(archived_at);
        let archived = source.clone();
        self.commit(&next)?;

        Ok(archived)
    }

    /// Save or replace a cited source span.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStoreError`] for validation or commit failures.
    pub fn save_reference_span(
        &mut self,
        reference: ReferenceSpan,
    ) -> Result<ReferenceSpan, BetaStoreError> {
        assert_non_blank(&reference.id, "Reference span id")?;
        assert_known_source(&self.data, &reference.source_document_id)?;
        assert_non_blank(&reference.text, "Reference span text")?;
        let mut next = self.data.clone();
        upsert_by_id(&mut next.reference_spans, reference.clone());
        self.commit(&next)?;

        Ok(reference)
    }

    /// Save or replace a generation run.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStoreError`] for validation or commit failures.
    pub fn save_generation_run(
        &mut self,
        run: GenerationRun,
    ) -> Result<GenerationRun, BetaStoreError> {
        assert_non_blank(&run.id, "Generation run id")?;
        for source_document_id in &run.source_document_ids {
            assert_known_source(&self.data, source_document_id)?;
        }
        let mut next = self.data.clone();
        upsert_by_id(&mut next.generation_runs, run.clone());
        self.commit(&next)?;

        Ok(run)
    }

    /// Append one learner content judgment, preserving every prior revision.
    /// Replaying an existing id returns the original row without adding a
    /// duplicate; a new row may supersede an earlier id for latest-wins reads.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStoreError`] when the review unit is unknown, a replay
    /// conflicts with an existing id, or the snapshot cannot be committed.
    pub fn record_content_feedback(
        &mut self,
        feedback: ContentFeedback,
    ) -> Result<ContentFeedback, BetaStoreError> {
        self.transact(|snapshot| {
            assert_known_review_unit(snapshot, &feedback.review_unit_id)?;
            if let Some(existing) = snapshot
                .content_feedback
                .iter()
                .find(|existing| existing.id == feedback.id)
            {
                if content_feedback_replay_matches(existing, &feedback) {
                    return Ok(existing.clone());
                }
                return Err(BetaStoreError::DuplicateContentFeedback(
                    feedback.id.clone(),
                ));
            }

            if let Some(supersedes_id) = &feedback.supersedes_id {
                let superseded = snapshot
                    .content_feedback
                    .iter()
                    .find(|existing| &existing.id == supersedes_id)
                    .ok_or_else(|| {
                        BetaStoreError::FeedbackSupersedesUnknown(supersedes_id.clone())
                    })?;
                if superseded.review_unit_id != feedback.review_unit_id {
                    return Err(BetaStoreError::FeedbackSupersedesOtherReviewUnit(
                        supersedes_id.clone(),
                    ));
                }
                if superseded.account_id != feedback.account_id {
                    return Err(BetaStoreError::FeedbackSupersedesOtherAccount(
                        supersedes_id.clone(),
                    ));
                }
            }
            let current_head = current_feedback_head(
                &snapshot.content_feedback,
                &feedback.account_id,
                &feedback.review_unit_id,
            );
            if feedback.supersedes_id != current_head.as_ref().map(|row| row.id.clone()) {
                return Err(BetaStoreError::FeedbackSupersedesStale {
                    expected_head: current_head.map(|row| row.id.clone()),
                    supplied_parent: feedback.supersedes_id.clone(),
                });
            }

            snapshot.content_feedback.push(feedback.clone());
            Ok(feedback)
        })
    }

    /// Resolve active feedback rows to the generation configuration that
    /// produced each card and emit the bench calibration label shape.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStoreError`] when a feedback row cannot resolve its
    /// review unit, draft, or generation run provenance.
    pub fn export_content_feedback(&self) -> Result<Vec<ContentFeedbackExport>, BetaStoreError> {
        export_content_feedback(&self.snapshot())
    }

    /// Serialize the active export so the output can be passed directly to
    /// `memory-engine-bench calibrate --labels`.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStoreError`] when provenance resolution or JSON
    /// serialization fails.
    pub fn export_content_feedback_json(&self) -> Result<String, BetaStoreError> {
        serde_json::to_string_pretty(&self.export_content_feedback()?).map_err(BetaStoreError::Json)
    }

    /// Save or replace a generated concept-level reference note.
    ///
    /// Concept notes are not source evidence. They cache provider-written
    /// explanations for items that have no source span, and generated bridge
    /// drafts may cite them instead of a [`ReferenceSpan`].
    ///
    /// # Errors
    ///
    /// Returns [`BetaStoreError`] for validation or commit failures.
    pub fn save_concept_reference_note(
        &mut self,
        note: ConceptReferenceNote,
    ) -> Result<ConceptReferenceNote, BetaStoreError> {
        assert_concept_reference_note_contract(&note)?;
        let mut next = self.data.clone();
        upsert_concept_reference_note(&mut next.concept_reference_notes, note.clone());
        self.commit(&next)?;

        Ok(note)
    }

    /// Save or replace a generated prompt draft.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStoreError`] for validation or commit failures.
    pub fn save_generated_prompt_draft(
        &mut self,
        draft: GeneratedPromptDraft,
    ) -> Result<GeneratedPromptDraft, BetaStoreError> {
        assert_draft_contract(&self.data, &draft)?;
        let mut next = self.data.clone();
        upsert_by_id(&mut next.generated_prompt_drafts, draft.clone());
        self.commit(&next)?;

        Ok(draft)
    }

    /// Promote an accepted generated draft into a review unit.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStoreError`] for validation or commit failures.
    pub fn approve_generated_prompt_draft(
        &mut self,
        draft_id: &str,
        options: ApproveGeneratedPromptDraftOptions,
    ) -> Result<BetaReviewUnitRecord, BetaStoreError> {
        let draft = find_by_id(&self.data.generated_prompt_drafts, draft_id)
            .ok_or_else(|| BetaStoreError::UnknownGeneratedPromptDraft(draft_id.to_owned()))?;
        if draft.validation.status != GeneratedPromptValidationStatus::Accepted {
            return Err(BetaStoreError::RejectedGeneratedPromptDraft);
        }
        if draft
            .generation_run_id
            .as_ref()
            .is_none_or(|run_id| find_by_id(&self.data.generation_runs, run_id).is_none())
        {
            return Err(BetaStoreError::MissingGenerationRunForAcceptedDraft);
        }

        if let Some(existing) = self
            .data
            .review_units
            .iter()
            .find(|unit| unit.review_unit_id == draft.review_unit_id)
        {
            return Ok(existing.clone());
        }

        let review_unit = BetaReviewUnitRecord {
            review_unit_id: draft.review_unit_id.clone(),
            prompt_id: draft.prompt_id.clone(),
            prompt: draft.prompt.clone(),
            queue: draft.queue.clone(),
            reference_span_ids: draft.reference_span_ids.clone(),
            concept_reference_note_key: draft.concept_reference_note_key.clone(),
            generated_prompt_draft_id: Some(draft.id.clone()),
            archived_at: None,
            snoozed_until: None,
            created_at: draft.created_at,
        };
        let mut next = self.data.clone();
        next.review_units.push(review_unit.clone());
        apply_schedule_record(
            &mut next,
            &draft.review_unit_id,
            options.initial_schedule_state,
        );
        self.commit(&next)?;

        Ok(review_unit)
    }

    /// Replace an approved review unit's prompt text while preserving its answer contract.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStoreError`] when the review unit is unknown, archived, or invalid.
    pub fn update_review_unit_prompt_text(
        &mut self,
        review_unit_id: &ReviewUnitId,
        prompt_text: &str,
        expected_answer: &str,
    ) -> Result<BetaReviewUnitRecord, BetaStoreError> {
        assert_non_blank(prompt_text, "Review unit prompt")?;
        assert_non_blank(expected_answer, "Review unit expected answer")?;
        let mut next = self.data.clone();
        let review_unit = next
            .review_units
            .iter_mut()
            .find(|unit| &unit.review_unit_id == review_unit_id)
            .ok_or_else(|| BetaStoreError::UnknownReviewUnit(review_unit_id.clone()))?;
        if review_unit.archived_at.is_some() {
            return Err(BetaStoreError::ReviewUnitArchived(review_unit_id.clone()));
        }

        replace_prompt_text(&mut review_unit.prompt, prompt_text);
        replace_prompt_answer(&mut review_unit.prompt, expected_answer);
        if let Some(draft_id) = &review_unit.generated_prompt_draft_id {
            if let Some(draft) = next
                .generated_prompt_drafts
                .iter_mut()
                .find(|draft| &draft.id == draft_id)
            {
                replace_prompt_text(&mut draft.prompt, prompt_text);
                replace_prompt_answer(&mut draft.prompt, expected_answer);
                if !draft
                    .critique_notes
                    .iter()
                    .any(|note| note == "Learner edited approved wording.")
                {
                    draft
                        .critique_notes
                        .push("Learner edited approved wording.".to_owned());
                }
            }
        }
        let updated = review_unit.clone();
        assert_review_unit_contract(&next, &updated)?;
        self.commit(&next)?;

        Ok(updated)
    }

    /// Hide an approved review unit from the active queue while preserving receipts.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStoreError`] when the review unit is unknown.
    pub fn archive_review_unit(
        &mut self,
        review_unit_id: &ReviewUnitId,
        archived_at: i64,
    ) -> Result<BetaReviewUnitRecord, BetaStoreError> {
        let mut next = self.data.clone();
        let review_unit = next
            .review_units
            .iter_mut()
            .find(|unit| &unit.review_unit_id == review_unit_id)
            .ok_or_else(|| BetaStoreError::UnknownReviewUnit(review_unit_id.clone()))?;
        review_unit.archived_at = Some(archived_at);
        let archived = review_unit.clone();
        self.commit(&next)?;

        Ok(archived)
    }

    /// Move an approved review unit's beta-owned queue availability forward.
    ///
    /// This does not mutate FSRS schedule fields; reviewed units keep their
    /// schedule record and expose the snoozed due through the queue candidate.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStoreError`] when the review unit is unknown or archived.
    pub fn snooze_review_unit_until(
        &mut self,
        review_unit_id: &ReviewUnitId,
        snoozed_until: i64,
    ) -> Result<BetaReviewUnitRecord, BetaStoreError> {
        let mut next = self.data.clone();
        let review_unit = next
            .review_units
            .iter_mut()
            .find(|unit| &unit.review_unit_id == review_unit_id)
            .ok_or_else(|| BetaStoreError::UnknownReviewUnit(review_unit_id.clone()))?;
        if review_unit.archived_at.is_some() {
            return Err(BetaStoreError::ReviewUnitArchived(review_unit_id.clone()));
        }
        review_unit.snoozed_until = Some(snoozed_until);
        let snoozed = review_unit.clone();
        self.commit(&next)?;

        Ok(snoozed)
    }

    /// Move every non-archived review unit under one persisted concept key
    /// forward in one file-store commit.
    ///
    /// This does not mutate schedule or attempt history. The returned records
    /// are the exact members changed by the commit.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStoreError`] when the single snapshot commit fails.
    pub fn snooze_review_units_for_concept_until(
        &mut self,
        concept_key: &str,
        snoozed_until: i64,
    ) -> Result<Vec<BetaReviewUnitRecord>, BetaStoreError> {
        let mut next = self.data.clone();
        let snoozed = next
            .review_units
            .iter_mut()
            .filter(|review_unit| review_unit.archived_at.is_none())
            .filter(|review_unit| review_unit.queue.concept_key.as_deref() == Some(concept_key))
            .map(|review_unit| {
                review_unit.snoozed_until = Some(snoozed_until);
                review_unit.clone()
            })
            .collect::<Vec<_>>();

        if snoozed.is_empty() {
            return Ok(snoozed);
        }

        self.commit(next)?;
        Ok(snoozed)
    }

    /// Replace an approved review unit's volatile lifecycle metadata.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStoreError`] when the review unit is unknown or archived.
    pub fn set_review_unit_lifecycle(
        &mut self,
        review_unit_id: &ReviewUnitId,
        lifecycle: ReviewUnitLifecycle,
    ) -> Result<BetaReviewUnitRecord, BetaStoreError> {
        let mut next = self.data.clone();
        let review_unit = next
            .review_units
            .iter_mut()
            .find(|unit| &unit.review_unit_id == review_unit_id)
            .ok_or_else(|| BetaStoreError::UnknownReviewUnit(review_unit_id.clone()))?;
        if review_unit.archived_at.is_some() {
            return Err(BetaStoreError::ReviewUnitArchived(review_unit_id.clone()));
        }
        review_unit.queue.lifecycle = lifecycle;
        let updated = review_unit.clone();
        self.commit(&next)?;

        Ok(updated)
    }

    /// Create a beta review unit, or return the existing persisted unit untouched.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStoreError`] for validation or commit failures.
    pub fn save_review_unit(
        &mut self,
        review_unit: BetaReviewUnitRecord,
    ) -> Result<BetaReviewUnitRecord, BetaStoreError> {
        if let Some(existing) = self
            .data
            .review_units
            .iter()
            .find(|unit| unit.review_unit_id == review_unit.review_unit_id)
        {
            return Ok(existing.clone());
        }
        assert_review_unit_contract(&self.data, &review_unit)?;
        let mut next = self.data.clone();
        upsert_review_unit(&mut next.review_units, review_unit.clone());
        self.commit(&next)?;

        Ok(review_unit)
    }

    /// Set or clear schedule state for a known review unit.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStoreError`] for validation or commit failures.
    pub fn set_schedule_state(
        &mut self,
        review_unit_id: &ReviewUnitId,
        schedule_state: Option<ScheduleState>,
    ) -> Result<(), BetaStoreError> {
        assert_known_review_unit(&self.data, review_unit_id)?;
        let mut next = self.data.clone();
        apply_schedule_record(&mut next, review_unit_id, schedule_state);
        self.commit(&next)
    }

    fn commit(&mut self, next: &BetaStoreSnapshot) -> Result<(), BetaStoreError> {
        if self.fail_next_commit {
            self.fail_next_commit = false;
            return Err(BetaStoreError::InjectedCommitFailure);
        }

        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let _lock = StoreFileLock::acquire(&self.path)?;
        let latest = load_snapshot(&self.path)?;
        let next = merge_snapshot(&self.data, next, latest);
        persist_snapshot(&self.path, &next)?;
        self.data = next;

        Ok(())
    }

    fn transact<T>(
        &mut self,
        operation: impl FnOnce(&mut BetaStoreSnapshot) -> Result<T, BetaStoreError>,
    ) -> Result<T, BetaStoreError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let _lock = StoreFileLock::acquire(&self.path)?;
        let mut snapshot = load_snapshot(&self.path)?;
        let result = operation(&mut snapshot)?;
        if self.fail_next_commit {
            self.fail_next_commit = false;
            return Err(BetaStoreError::InjectedCommitFailure);
        }
        persist_snapshot(&self.path, &snapshot)?;
        self.data = snapshot;
        Ok(result)
    }
}

impl MemoryServiceStore for BetaPersistenceStore {
    type Error = BetaStoreError;

    fn record_attempt(&mut self, attempt: ServiceAttemptRecord) -> Result<(), Self::Error> {
        assert_attempt_contract(&self.data, &attempt)?;
        let mut next = self.data.clone();
        next.attempts.push(attempt);
        self.commit(&next)
    }

    fn read_schedule_state(
        &self,
        review_unit_id: &ReviewUnitId,
    ) -> Result<Option<ScheduleState>, Self::Error> {
        assert_known_review_unit(&self.data, review_unit_id)?;

        Ok(find_schedule(&self.data, review_unit_id).map(|record| record.state.clone()))
    }

    fn apply_review(
        &mut self,
        review_unit_id: &ReviewUnitId,
        attempt: ServiceAttemptRecord,
        schedule_state: ScheduleState,
        expected_prior_schedule_state: Option<ScheduleState>,
    ) -> Result<(), Self::Error> {
        if review_unit_id != &attempt.review_unit_id {
            return Err(BetaStoreError::ReviewUnitMismatch);
        }
        if schedule_state.last_review != Some(attempt.occurred_at) {
            return Err(BetaStoreError::ScheduleLastReviewMismatch);
        }

        let key = applied_review_key(&attempt);
        self.transact(|snapshot| {
            assert_known_review_unit(snapshot, review_unit_id)?;
            assert_attempt_contract(snapshot, &attempt)?;
            if snapshot
                .applied_reviews
                .iter()
                .any(|receipt| receipt.key == key)
            {
                return Err(BetaStoreError::DuplicateAppliedReview(key));
            }

            let current_schedule =
                find_schedule(snapshot, review_unit_id).map(|record| record.state.clone());
            if current_schedule != expected_prior_schedule_state {
                return Err(BetaStoreError::StaleScheduleWrite(review_unit_id.clone()));
            }

            snapshot.attempts.push(attempt.clone());
            apply_schedule_record(snapshot, review_unit_id, Some(schedule_state.clone()));
            snapshot.applied_reviews.push(AppliedReviewReceipt {
                key,
                attempt,
                expected_prior_schedule_state,
                schedule_state,
            });
            Ok(())
        })
    }

    fn list_queue_candidates(&self) -> Result<Vec<QueueCandidate>, Self::Error> {
        Ok(self
            .data
            .review_units
            .iter()
            .filter(|review_unit| review_unit.archived_at.is_none())
            .map(|review_unit| {
                let schedule_state = find_schedule(&self.data, &review_unit.review_unit_id)
                    .map(|record| record.state.clone());
                let mut candidate = review_unit.queue.with_schedule(schedule_state);
                if let Some(snoozed_until) = review_unit.snoozed_until {
                    candidate = defer_queue_availability(&candidate, snoozed_until);
                }
                candidate
            })
            .collect())
    }
}

/// Resolve a snapshot's active feedback rows to generation provenance and the
/// bench calibration label contract.
///
/// # Errors
///
/// Returns [`BetaStoreError`] when a feedback row cannot resolve its review
/// unit, draft, or generation run provenance.
pub fn export_content_feedback(
    snapshot: &BetaStoreSnapshot,
) -> Result<Vec<ContentFeedbackExport>, BetaStoreError> {
    snapshot
        .content_feedback
        .iter()
        .filter(|feedback| {
            current_feedback_head(
                &snapshot.content_feedback,
                &feedback.account_id,
                &feedback.review_unit_id,
            )
            .is_some_and(|head| head.id == feedback.id)
        })
        .map(|feedback| resolve_content_feedback(snapshot, feedback))
        .collect()
}

/// Serialize a snapshot's active feedback export for `bench calibrate`.
///
/// # Errors
///
/// Returns [`BetaStoreError`] when provenance resolution or JSON
/// serialization fails.
pub fn export_content_feedback_json(
    snapshot: &BetaStoreSnapshot,
) -> Result<String, BetaStoreError> {
    serde_json::to_string_pretty(&export_content_feedback(snapshot)?).map_err(BetaStoreError::Json)
}

impl ContentFeedbackStore for BetaPersistenceStore {
    type Error = BetaStoreError;

    fn record_content_feedback(
        &mut self,
        feedback: ContentFeedback,
    ) -> Result<ContentFeedback, Self::Error> {
        BetaPersistenceStore::record_content_feedback(self, feedback)
    }
}

fn resolve_content_feedback(
    snapshot: &BetaStoreSnapshot,
    feedback: &ContentFeedback,
) -> Result<ContentFeedbackExport, BetaStoreError> {
    let review_unit = snapshot
        .review_units
        .iter()
        .find(|unit| unit.review_unit_id == feedback.review_unit_id)
        .ok_or_else(|| {
            BetaStoreError::MissingFeedbackProvenance(feedback.review_unit_id.clone())
        })?;
    let draft_id = review_unit
        .generated_prompt_draft_id
        .as_ref()
        .ok_or_else(|| {
            BetaStoreError::MissingFeedbackProvenance(feedback.review_unit_id.clone())
        })?;
    let draft = snapshot
        .generated_prompt_drafts
        .iter()
        .find(|draft| &draft.id == draft_id)
        .ok_or_else(|| {
            BetaStoreError::MissingFeedbackProvenance(feedback.review_unit_id.clone())
        })?;
    let run_id = draft.generation_run_id.as_ref().ok_or_else(|| {
        BetaStoreError::MissingFeedbackProvenance(feedback.review_unit_id.clone())
    })?;
    let run = snapshot
        .generation_runs
        .iter()
        .find(|run| &run.id == run_id)
        .ok_or_else(|| {
            BetaStoreError::MissingFeedbackProvenance(feedback.review_unit_id.clone())
        })?;
    let human_keep = feedback.verdict == ContentFeedbackVerdict::Kept;

    Ok(ContentFeedbackExport {
        feedback_id: feedback.id.clone(),
        review_unit_id: feedback.review_unit_id.clone(),
        judge_keep: draft.validation.status == GeneratedPromptValidationStatus::Accepted,
        human_keep,
        question: prompt_question(&review_unit.prompt),
        rationale: feedback.rationale.clone(),
        gen_ai_system: run.provider.clone(),
        gen_ai_request_model: run.model.clone(),
        gen_ai_prompt_version: draft.model.version.clone(),
        gen_ai_evaluation_score_value: if human_keep { 1.0 } else { 0.0 },
        gen_ai_evaluation_explanation: feedback.rationale.clone(),
        fixture: (!human_keep).then(|| dropped_fixture(snapshot, review_unit, feedback)),
    })
}

fn prompt_question(prompt: &Prompt) -> String {
    match prompt {
        Prompt::Mcq { prompt, .. } | Prompt::Boolean { prompt, .. } => prompt.clone(),
        Prompt::Exact(prompt) => prompt.prompt.clone(),
    }
}

fn dropped_fixture(
    snapshot: &BetaStoreSnapshot,
    review_unit: &BetaReviewUnitRecord,
    feedback: &ContentFeedback,
) -> ContentFeedbackFixture {
    let source = review_unit
        .reference_span_ids
        .iter()
        .filter_map(|span_id| {
            snapshot
                .reference_spans
                .iter()
                .find(|span| &span.id == span_id)
        })
        .find_map(|span| {
            snapshot
                .source_documents
                .iter()
                .find(|source| source.id == span.source_document_id)
        })
        .or_else(|| {
            snapshot.source_documents.iter().find(|source| {
                review_unit.reference_span_ids.iter().any(|span_id| {
                    snapshot
                        .reference_spans
                        .iter()
                        .any(|span| &span.id == span_id && span.source_document_id == source.id)
                })
            })
        });
    let question = prompt_question(&review_unit.prompt);
    let (title, body) = match source {
        Some(source) if matches!(source.permission, SourcePermission::ModelEligible) => (
            source.title.clone(),
            source.body.clone().unwrap_or_else(|| question.clone()),
        ),
        Some(_) => (
            "[redacted local-only source]".to_owned(),
            "[redacted local-only source]".to_owned(),
        ),
        None => (question.clone(), question.clone()),
    };

    let activity_kind = snapshot
        .generated_prompt_drafts
        .iter()
        .find(|draft| review_unit.generated_prompt_draft_id.as_ref() == Some(&draft.id))
        .map_or("quiz", |draft| match draft.activity_kind {
            GeneratedLearningActivityKind::Quiz => "quiz",
            GeneratedLearningActivityKind::Exercise => "exercise",
        });

    ContentFeedbackFixture {
        id: format!("feedback-dropped-{}", feedback.id),
        title,
        category: "content-feedback-dropped".to_owned(),
        body,
        expect: ContentFeedbackFixtureExpect {
            min_drafts: 1,
            max_drafts: 1,
            key_terms: Vec::new(),
            intent: "concept_understanding".to_owned(),
            required_activity_kinds: vec![activity_kind.to_owned()],
            required_activity_stage_terms: Vec::new(),
        },
    }
}

fn load_snapshot(path: &Path) -> Result<BetaStoreSnapshot, BetaStoreError> {
    if !path.exists() {
        return Ok(BetaStoreSnapshot::default());
    }

    let parsed = serde_json::from_str::<BetaStoreSnapshot>(&fs::read_to_string(path)?)?;
    if parsed.version != 1 {
        return Err(BetaStoreError::UnsupportedVersion(parsed.version));
    }

    Ok(parsed)
}

fn persist_snapshot(path: &Path, snapshot: &BetaStoreSnapshot) -> Result<(), BetaStoreError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary_path = temporary_path(path);
    let encoded = serde_json::to_string_pretty(snapshot)?;
    fs::write(&temporary_path, format!("{encoded}\n"))?;
    fs::rename(&temporary_path, path)?;
    Ok(())
}

struct StoreFileLock {
    file: fs::File,
}

impl StoreFileLock {
    fn acquire(store_path: &Path) -> Result<Self, BetaStoreError> {
        let path = store_path.with_extension("lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        for _ in 0..5_000 {
            match file.try_lock() {
                Ok(()) => return Ok(Self { file }),
                Err(fs::TryLockError::WouldBlock) => {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Err(fs::TryLockError::Error(error))
                    if error.kind() == io::ErrorKind::Interrupted => {}
                Err(fs::TryLockError::Error(error)) => return Err(BetaStoreError::Io(error)),
            }
        }
        Err(BetaStoreError::Io(io::Error::new(
            io::ErrorKind::TimedOut,
            "timed out acquiring beta store lock",
        )))
    }
}

impl Drop for StoreFileLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

fn current_feedback_head<'a>(
    feedback: &'a [ContentFeedback],
    account_id: &str,
    review_unit_id: &ReviewUnitId,
) -> Option<&'a ContentFeedback> {
    let superseded: BTreeSet<&str> = feedback
        .iter()
        .filter(|row| row.account_id == account_id && row.review_unit_id == *review_unit_id)
        .filter_map(|row| row.supersedes_id.as_deref())
        .collect();
    feedback
        .iter()
        .filter(|row| {
            row.account_id == account_id
                && row.review_unit_id == *review_unit_id
                && !superseded.contains(row.id.as_str())
        })
        .max_by_key(|row| (row.occurred_at, row.id.as_str()))
}

fn merge_snapshot(
    base: &BetaStoreSnapshot,
    proposed: &BetaStoreSnapshot,
    mut latest: BetaStoreSnapshot,
) -> BetaStoreSnapshot {
    merge_upserted(
        &mut latest.source_documents,
        &base.source_documents,
        &proposed.source_documents,
        |row| row.id.clone(),
    );
    merge_upserted(
        &mut latest.reference_spans,
        &base.reference_spans,
        &proposed.reference_spans,
        |row| row.id.clone(),
    );
    merge_upserted(
        &mut latest.generated_prompt_drafts,
        &base.generated_prompt_drafts,
        &proposed.generated_prompt_drafts,
        |row| row.id.clone(),
    );
    merge_upserted(
        &mut latest.review_units,
        &base.review_units,
        &proposed.review_units,
        |row| row.review_unit_id.clone(),
    );
    merge_upserted(
        &mut latest.generation_runs,
        &base.generation_runs,
        &proposed.generation_runs,
        |row| row.id.clone(),
    );
    merge_upserted(
        &mut latest.concept_reference_notes,
        &base.concept_reference_notes,
        &proposed.concept_reference_notes,
        |row| row.concept_key.clone(),
    );
    merge_appended(&mut latest.attempts, &base.attempts, &proposed.attempts);
    merge_appended(
        &mut latest.content_feedback,
        &base.content_feedback,
        &proposed.content_feedback,
    );
    merge_appended(
        &mut latest.applied_reviews,
        &base.applied_reviews,
        &proposed.applied_reviews,
    );
    merge_schedules(&mut latest.schedules, &base.schedules, &proposed.schedules);
    latest
}

fn merge_upserted<T: Clone + PartialEq, K: Eq>(
    latest: &mut Vec<T>,
    base: &[T],
    proposed: &[T],
    key: impl Fn(&T) -> K,
) {
    for row in proposed {
        let row_key = key(row);
        let changed = base.iter().find(|old| key(old) == row_key) != Some(row);
        if changed {
            if let Some(existing) = latest.iter_mut().find(|old| key(old) == row_key) {
                *existing = row.clone();
            } else {
                latest.push(row.clone());
            }
        }
    }
}

fn merge_appended<T: Clone + PartialEq>(latest: &mut Vec<T>, base: &[T], proposed: &[T]) {
    for row in proposed.iter().filter(|row| !base.contains(row)) {
        if !latest.contains(row) {
            latest.push(row.clone());
        }
    }
}

fn merge_schedules(
    latest: &mut Vec<ScheduleRecord>,
    base: &[ScheduleRecord],
    proposed: &[ScheduleRecord],
) {
    for old in base {
        let before = Some(old);
        let after = proposed
            .iter()
            .find(|row| row.review_unit_id == old.review_unit_id);
        if before != after {
            latest.retain(|row| row.review_unit_id != old.review_unit_id);
            if let Some(after) = after {
                latest.push(after.clone());
            }
        }
    }
    for row in proposed.iter().filter(|row| {
        !base
            .iter()
            .any(|old| old.review_unit_id == row.review_unit_id)
    }) {
        if let Some(existing) = latest
            .iter_mut()
            .find(|old| old.review_unit_id == row.review_unit_id)
        {
            *existing = row.clone();
        } else {
            latest.push(row.clone());
        }
    }
}

fn temporary_path(path: &Path) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".{stamp}.tmp"));
    PathBuf::from(temporary)
}

fn assert_non_blank(value: &str, label: &'static str) -> Result<(), BetaStoreError> {
    if value.trim().is_empty() {
        Err(BetaStoreError::Blank { label })
    } else {
        Ok(())
    }
}

fn assert_known_source(
    snapshot: &BetaStoreSnapshot,
    source_document_id: &str,
) -> Result<(), BetaStoreError> {
    if find_by_id(&snapshot.source_documents, source_document_id).is_some() {
        Ok(())
    } else {
        Err(BetaStoreError::UnknownSourceDocument(
            source_document_id.to_owned(),
        ))
    }
}

fn assert_known_reference(
    snapshot: &BetaStoreSnapshot,
    reference_span_id: &str,
) -> Result<(), BetaStoreError> {
    if find_by_id(&snapshot.reference_spans, reference_span_id).is_some() {
        Ok(())
    } else {
        Err(BetaStoreError::UnknownReferenceSpan(
            reference_span_id.to_owned(),
        ))
    }
}

fn assert_known_concept_reference_note(
    snapshot: &BetaStoreSnapshot,
    concept_key: &str,
) -> Result<(), BetaStoreError> {
    if snapshot
        .concept_reference_notes
        .iter()
        .any(|note| note.concept_key == concept_key)
    {
        Ok(())
    } else {
        Err(BetaStoreError::UnknownConceptReferenceNote(
            concept_key.to_owned(),
        ))
    }
}

fn assert_known_review_unit(
    snapshot: &BetaStoreSnapshot,
    review_unit_id: &ReviewUnitId,
) -> Result<(), BetaStoreError> {
    if snapshot
        .review_units
        .iter()
        .any(|unit| &unit.review_unit_id == review_unit_id)
    {
        Ok(())
    } else {
        Err(BetaStoreError::UnknownReviewUnit(review_unit_id.clone()))
    }
}

fn assert_attempt_contract(
    snapshot: &BetaStoreSnapshot,
    attempt: &ServiceAttemptRecord,
) -> Result<(), BetaStoreError> {
    assert_known_review_unit(snapshot, &attempt.review_unit_id)?;
    if attempt.submitted_answer.trim().is_empty() {
        return Err(BetaStoreError::AttemptAnswerBlank);
    }
    if attempt.response_time_ms == 0 {
        return Err(BetaStoreError::AttemptResponseTimeNonPositive);
    }

    Ok(())
}

fn assert_draft_contract(
    snapshot: &BetaStoreSnapshot,
    draft: &GeneratedPromptDraft,
) -> Result<(), BetaStoreError> {
    assert_non_blank(&draft.id, "Generated prompt draft id")?;
    assert_non_blank(&draft.prompt_id, "Generated prompt draft prompt id")?;
    assert_non_blank(
        &draft.activity_stage,
        "Generated prompt draft activity stage",
    )?;
    assert_non_blank(&draft.model.provider, "Generated prompt draft provider")?;
    assert_non_blank(&draft.model.name, "Generated prompt draft model")?;
    assert_non_blank(&draft.model.version, "Generated prompt draft model version")?;
    let cites_concept_note = draft.concept_reference_note_key.is_some();
    let provider_generated = draft.generation_run_id.is_some();
    if provider_generated && draft.source_document_ids.is_empty() && !cites_concept_note {
        return Err(BetaStoreError::GeneratedPromptDraftRequiresSource);
    }
    if provider_generated && !cites_concept_note && draft.reference_span_ids.is_empty() {
        return Err(BetaStoreError::GeneratedPromptDraftRequiresReference);
    }
    if provider_generated && cites_concept_note && !draft.source_document_ids.is_empty() {
        return Err(BetaStoreError::GeneratedPromptDraftRequiresReference);
    }
    if prompt_review_unit_id(&draft.prompt) != &draft.review_unit_id
        || draft.queue.review_unit_id != draft.review_unit_id
    {
        return Err(BetaStoreError::GeneratedPromptDraftReviewUnitMismatch);
    }
    for source_document_id in &draft.source_document_ids {
        assert_known_source(snapshot, source_document_id)?;
    }
    for reference_span_id in &draft.reference_span_ids {
        assert_known_reference(snapshot, reference_span_id)?;
    }
    if let Some(concept_key) = &draft.concept_reference_note_key {
        assert_known_concept_reference_note(snapshot, concept_key)?;
    }

    Ok(())
}

fn assert_review_unit_contract(
    snapshot: &BetaStoreSnapshot,
    review_unit: &BetaReviewUnitRecord,
) -> Result<(), BetaStoreError> {
    if prompt_review_unit_id(&review_unit.prompt) != &review_unit.review_unit_id
        || review_unit.queue.review_unit_id != review_unit.review_unit_id
    {
        return Err(BetaStoreError::ReviewUnitMismatch);
    }
    for reference_span_id in &review_unit.reference_span_ids {
        assert_known_reference(snapshot, reference_span_id)?;
    }
    if let Some(concept_key) = &review_unit.concept_reference_note_key {
        assert_known_concept_reference_note(snapshot, concept_key)?;
    }
    if let Some(draft_id) = &review_unit.generated_prompt_draft_id {
        if find_by_id(&snapshot.generated_prompt_drafts, draft_id).is_none() {
            return Err(BetaStoreError::UnknownGeneratedPromptDraft(
                draft_id.clone(),
            ));
        }
    }

    Ok(())
}

fn assert_concept_reference_note_contract(
    note: &ConceptReferenceNote,
) -> Result<(), BetaStoreError> {
    assert_non_blank(&note.concept_key, "Concept reference note key")?;
    assert_non_blank(&note.title, "Concept reference note title")?;
    assert_non_blank(&note.body, "Concept reference note body")?;
    assert_non_blank(&note.model.provider, "Concept reference note provider")?;
    assert_non_blank(&note.model.name, "Concept reference note model")?;
    assert_non_blank(&note.model.version, "Concept reference note model version")
}

fn prompt_review_unit_id(prompt: &Prompt) -> &ReviewUnitId {
    match prompt {
        Prompt::Mcq { review_unit_id, .. } | Prompt::Boolean { review_unit_id, .. } => {
            review_unit_id
        }
        Prompt::Exact(prompt) => &prompt.review_unit_id,
    }
}

fn replace_prompt_text(prompt: &mut Prompt, prompt_text: &str) {
    match prompt {
        Prompt::Mcq { prompt, .. } | Prompt::Boolean { prompt, .. } => {
            prompt_text.clone_into(prompt);
        }
        Prompt::Exact(prompt) => {
            prompt_text.clone_into(&mut prompt.prompt);
        }
    }
}

fn replace_prompt_answer(prompt: &mut Prompt, expected_answer: &str) {
    match prompt {
        Prompt::Mcq {
            choices,
            correct_choice,
            ..
        } => {
            expected_answer.clone_into(correct_choice);
            if !choices.iter().any(|choice| choice == expected_answer) {
                choices.push(expected_answer.to_owned());
            }
        }
        Prompt::Boolean { correct_answer, .. } => {
            *correct_answer = expected_answer.eq_ignore_ascii_case("true");
        }
        Prompt::Exact(prompt) => {
            prompt.accepted_answers = vec![expected_answer.to_owned()];
        }
    }
}

trait HasId {
    fn id(&self) -> &str;
}

impl HasId for SourceDocument {
    fn id(&self) -> &str {
        &self.id
    }
}

impl HasId for ReferenceSpan {
    fn id(&self) -> &str {
        &self.id
    }
}

impl HasId for GeneratedPromptDraft {
    fn id(&self) -> &str {
        &self.id
    }
}

impl HasId for GenerationRun {
    fn id(&self) -> &str {
        &self.id
    }
}

fn upsert_concept_reference_note(
    items: &mut Vec<ConceptReferenceNote>,
    item: ConceptReferenceNote,
) {
    if let Some(index) = items
        .iter()
        .position(|candidate| candidate.concept_key == item.concept_key)
    {
        items[index] = item;
    } else {
        items.push(item);
    }
}

fn upsert_by_id<T: HasId>(items: &mut Vec<T>, item: T) {
    if let Some(index) = items
        .iter()
        .position(|candidate| candidate.id() == item.id())
    {
        items[index] = item;
    } else {
        items.push(item);
    }
}

fn find_by_id<'a, T: HasId>(items: &'a [T], id: &str) -> Option<&'a T> {
    items.iter().find(|item| item.id() == id)
}

fn find_schedule<'a>(
    snapshot: &'a BetaStoreSnapshot,
    review_unit_id: &ReviewUnitId,
) -> Option<&'a ScheduleRecord> {
    snapshot
        .schedules
        .iter()
        .find(|schedule| &schedule.review_unit_id == review_unit_id)
}

fn apply_schedule_record(
    snapshot: &mut BetaStoreSnapshot,
    review_unit_id: &ReviewUnitId,
    schedule_state: Option<ScheduleState>,
) {
    let existing_index = snapshot
        .schedules
        .iter()
        .position(|schedule| &schedule.review_unit_id == review_unit_id);

    let Some(schedule_state) = schedule_state else {
        if let Some(index) = existing_index {
            snapshot.schedules.remove(index);
        }
        return;
    };

    let record = ScheduleRecord {
        review_unit_id: review_unit_id.clone(),
        state: schedule_state,
    };
    if let Some(index) = existing_index {
        snapshot.schedules[index] = record;
    } else {
        snapshot.schedules.push(record);
    }
}

fn applied_review_key(attempt: &ServiceAttemptRecord) -> String {
    if let Some(idempotency_key) = &attempt.idempotency_key {
        return format!("idempotency:{idempotency_key}");
    }

    [
        "attempt".to_owned(),
        attempt.review_unit_id.to_string(),
        attempt.prompt_id.clone().unwrap_or_default(),
        attempt.submitted_answer.clone(),
        attempt.response_time_ms.to_string(),
        attempt.occurred_at.to_string(),
    ]
    .join("\0")
}
