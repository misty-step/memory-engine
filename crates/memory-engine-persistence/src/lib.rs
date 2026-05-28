//! File-backed beta persistence store for memory-engine.
//!
//! This crate owns durable beta-study state and implements the service store
//! trait. It intentionally keeps filesystem details here, outside the pure core
//! and service orchestration crates.

use std::{
    error::Error,
    fmt, fs, io,
    path::{Path, PathBuf},
};

use memory_engine_core::{Prompt, QueueCandidate, ReviewUnitId, ScheduleState};
use memory_engine_service::{MemoryServiceStore, ServiceAttemptRecord};
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
    pub body: Option<String>,
    pub uri: Option<String>,
    pub permission: SourcePermission,
    pub freshness: Option<i64>,
    pub created_at: i64,
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
    pub draft_ids: Vec<String>,
    pub provider: String,
    pub model: String,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub validation_failures: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BetaReviewUnitRecord {
    pub review_unit_id: ReviewUnitId,
    pub prompt_id: String,
    pub prompt: Prompt,
    pub queue: PersistedQueueCandidate,
    pub reference_span_ids: Vec<String>,
    pub generated_prompt_draft_id: Option<String>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedQueueCandidate {
    pub review_unit_id: ReviewUnitId,
    pub due: i64,
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
    pub applied_reviews: Vec<AppliedReviewReceipt>,
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
            applied_reviews: Vec::new(),
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
    Blank { label: &'static str },
    UnknownSourceDocument(String),
    UnknownReferenceSpan(String),
    UnknownReviewUnit(ReviewUnitId),
    UnknownGeneratedPromptDraft(String),
    RejectedGeneratedPromptDraft,
    MissingGenerationRunForAcceptedDraft,
    GeneratedPromptDraftRequiresSource,
    GeneratedPromptDraftRequiresReference,
    GeneratedPromptDraftReviewUnitMismatch,
    ReviewUnitMismatch,
    AttemptAnswerBlank,
    AttemptResponseTimeNonPositive,
    ScheduleLastReviewMismatch,
    DuplicateAppliedReview(String),
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
            | (Self::UnknownGeneratedPromptDraft(left), Self::UnknownGeneratedPromptDraft(right))
            | (Self::DuplicateAppliedReview(left), Self::DuplicateAppliedReview(right)) => {
                left == right
            }
            (Self::UnknownReviewUnit(left), Self::UnknownReviewUnit(right))
            | (Self::StaleScheduleWrite(left), Self::StaleScheduleWrite(right)) => left == right,
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
        self.data.clone()
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
        self.commit(next)?;

        Ok(document)
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
        self.commit(next)?;

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
        self.commit(next)?;

        Ok(run)
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
        self.commit(next)?;

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

        let review_unit = BetaReviewUnitRecord {
            review_unit_id: draft.review_unit_id.clone(),
            prompt_id: draft.prompt_id.clone(),
            prompt: draft.prompt.clone(),
            queue: draft.queue.clone(),
            reference_span_ids: draft.reference_span_ids.clone(),
            generated_prompt_draft_id: Some(draft.id.clone()),
            created_at: draft.created_at,
        };
        let mut next = self.data.clone();
        upsert_review_unit(&mut next.review_units, review_unit.clone());
        apply_schedule_record(
            &mut next,
            &draft.review_unit_id,
            options.initial_schedule_state,
        );
        self.commit(next)?;

        Ok(review_unit)
    }

    /// Save or replace a beta review unit.
    ///
    /// # Errors
    ///
    /// Returns [`BetaStoreError`] for validation or commit failures.
    pub fn save_review_unit(
        &mut self,
        review_unit: BetaReviewUnitRecord,
    ) -> Result<BetaReviewUnitRecord, BetaStoreError> {
        assert_review_unit_contract(&self.data, &review_unit)?;
        let mut next = self.data.clone();
        upsert_review_unit(&mut next.review_units, review_unit.clone());
        self.commit(next)?;

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
        self.commit(next)
    }

    fn commit(&mut self, next: BetaStoreSnapshot) -> Result<(), BetaStoreError> {
        if self.fail_next_commit {
            self.fail_next_commit = false;
            return Err(BetaStoreError::InjectedCommitFailure);
        }

        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary_path = temporary_path(&self.path);
        let encoded = serde_json::to_string_pretty(&next)?;
        fs::write(&temporary_path, format!("{encoded}\n"))?;
        fs::rename(&temporary_path, &self.path)?;
        self.data = next;

        Ok(())
    }
}

impl MemoryServiceStore for BetaPersistenceStore {
    type Error = BetaStoreError;

    fn record_attempt(&mut self, attempt: ServiceAttemptRecord) -> Result<(), Self::Error> {
        assert_attempt_contract(&self.data, &attempt)?;
        let mut next = self.data.clone();
        next.attempts.push(attempt);
        self.commit(next)
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
        assert_known_review_unit(&self.data, review_unit_id)?;
        assert_attempt_contract(&self.data, &attempt)?;
        if review_unit_id != &attempt.review_unit_id {
            return Err(BetaStoreError::ReviewUnitMismatch);
        }
        if schedule_state.last_review != Some(attempt.occurred_at) {
            return Err(BetaStoreError::ScheduleLastReviewMismatch);
        }

        let key = applied_review_key(&attempt);
        if self
            .data
            .applied_reviews
            .iter()
            .any(|receipt| receipt.key == key)
        {
            return Err(BetaStoreError::DuplicateAppliedReview(key));
        }

        let current_schedule =
            find_schedule(&self.data, review_unit_id).map(|record| record.state.clone());
        if current_schedule != expected_prior_schedule_state {
            return Err(BetaStoreError::StaleScheduleWrite(review_unit_id.clone()));
        }

        let mut next = self.data.clone();
        next.attempts.push(attempt.clone());
        apply_schedule_record(&mut next, review_unit_id, Some(schedule_state.clone()));
        next.applied_reviews.push(AppliedReviewReceipt {
            key,
            attempt,
            expected_prior_schedule_state,
            schedule_state,
        });
        self.commit(next)
    }

    fn list_queue_candidates(&self) -> Result<Vec<QueueCandidate>, Self::Error> {
        Ok(self
            .data
            .review_units
            .iter()
            .map(|review_unit| {
                let schedule_state = find_schedule(&self.data, &review_unit.review_unit_id)
                    .map(|record| record.state.clone());
                review_unit.queue.with_schedule(schedule_state)
            })
            .collect())
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
    if draft.source_document_ids.is_empty() {
        return Err(BetaStoreError::GeneratedPromptDraftRequiresSource);
    }
    if draft.reference_span_ids.is_empty() {
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
    if let Some(draft_id) = &review_unit.generated_prompt_draft_id {
        if find_by_id(&snapshot.generated_prompt_drafts, draft_id).is_none() {
            return Err(BetaStoreError::UnknownGeneratedPromptDraft(
                draft_id.clone(),
            ));
        }
    }

    Ok(())
}

fn prompt_review_unit_id(prompt: &Prompt) -> &ReviewUnitId {
    match prompt {
        Prompt::Mcq { review_unit_id, .. } | Prompt::Boolean { review_unit_id, .. } => {
            review_unit_id
        }
        Prompt::Exact(prompt) => &prompt.review_unit_id,
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

fn upsert_review_unit(items: &mut Vec<BetaReviewUnitRecord>, item: BetaReviewUnitRecord) {
    if let Some(index) = items
        .iter()
        .position(|candidate| candidate.review_unit_id == item.review_unit_id)
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
