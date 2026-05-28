//! Rust authored-content import dogfood path.
//!
//! Product-authored source text, translations, confidence prompts, and notes
//! stay in this boundary. The reusable kernel only receives canonical prompts,
//! queue candidates, and schedule state.

use std::{collections::BTreeMap, error::Error, fmt};

use memory_engine_core::{
    ExactPrompt, ExactPromptKind, Prompt, QueueCandidate, ReviewUnitId, ScheduleState,
    ScheduleStatus, Verdict,
};
use memory_engine_service::{
    GradeApplyReviewCommand, MemoryService, MemoryServiceStore, NextQueueCommand, NextQueueOptions,
    ServiceAttemptRecord, ServiceError,
};
use serde::Serialize;

const NOW: i64 = 1_779_984_000_000;
const ONE_MINUTE_MS: i64 = 60_000;
const ONE_DAY_MS: i64 = 86_400_000;
const PUNCTUATION_TOKENS: [&str; 4] = [".", ",", ";", ":"];

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportProbeReceipt {
    pub fixture: String,
    pub prompts: usize,
    pub queue_candidates: usize,
    pub schedules: usize,
    pub product_owned_fields: Vec<String>,
    pub api_gap: Option<String>,
    pub graded_verdict: String,
    pub next_review_unit_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CompiledImportProbe {
    pub fixture: String,
    pub prompts: Vec<Prompt>,
    pub prompt_ids: Vec<(ReviewUnitId, String)>,
    pub queue: Vec<QueueCandidate>,
    pub schedules: Vec<CompiledSchedule>,
    pub product_owned_fields: Vec<String>,
    pub api_gap: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CompiledSchedule {
    pub review_unit_id: ReviewUnitId,
    pub state: ScheduleState,
}

#[derive(Clone, Debug)]
struct AuthoredFixture {
    id: &'static str,
    domain_key: &'static str,
    source_key: &'static str,
    cards: Vec<AuthoredCard>,
}

#[derive(Clone, Debug)]
struct AuthoredCard {
    id: &'static str,
    source_text: &'static str,
    translation: &'static str,
    concept_key: &'static str,
    stage: ImportStage,
    confidence_prompt: &'static str,
    notes: Vec<&'static str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ImportStage {
    New,
    Review,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImportProbeError {
    EmptyFixture,
    UnknownReviewUnit(ReviewUnitId),
    MissingPromptId(ReviewUnitId),
    Service(ServiceError<ImportStoreError>),
}

impl fmt::Display for ImportProbeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyFixture => formatter.write_str("import fixture must not be empty"),
            Self::UnknownReviewUnit(id) => write!(formatter, "Unknown review unit: {id}"),
            Self::MissingPromptId(id) => write!(formatter, "Missing prompt id: {id}"),
            Self::Service(error) => write!(formatter, "service error: {error}"),
        }
    }
}

impl Error for ImportProbeError {}

impl From<ServiceError<ImportStoreError>> for ImportProbeError {
    fn from(error: ServiceError<ImportStoreError>) -> Self {
        Self::Service(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImportStoreError {
    UnknownReviewUnit(ReviewUnitId),
}

impl fmt::Display for ImportStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownReviewUnit(id) => write!(formatter, "Unknown review unit: {id}"),
        }
    }
}

impl Error for ImportStoreError {}

#[derive(Clone, Debug)]
struct ImportProbeStore {
    prompts: BTreeMap<ReviewUnitId, Prompt>,
    queue: BTreeMap<ReviewUnitId, QueueCandidate>,
    attempts: Vec<ServiceAttemptRecord>,
    schedules: BTreeMap<ReviewUnitId, ScheduleState>,
}

impl ImportProbeStore {
    fn new(compiled: &CompiledImportProbe) -> Self {
        Self {
            prompts: compiled
                .prompts
                .iter()
                .map(|prompt| (prompt_review_unit_id(prompt).clone(), prompt.clone()))
                .collect(),
            queue: compiled
                .queue
                .iter()
                .map(|candidate| (candidate.review_unit_id.clone(), candidate.clone()))
                .collect(),
            attempts: Vec::new(),
            schedules: compiled
                .schedules
                .iter()
                .map(|schedule| (schedule.review_unit_id.clone(), schedule.state.clone()))
                .collect(),
        }
    }

    fn assert_known(&self, review_unit_id: &ReviewUnitId) -> Result<(), ImportStoreError> {
        if self.prompts.contains_key(review_unit_id) && self.queue.contains_key(review_unit_id) {
            Ok(())
        } else {
            Err(ImportStoreError::UnknownReviewUnit(review_unit_id.clone()))
        }
    }
}

impl MemoryServiceStore for ImportProbeStore {
    type Error = ImportStoreError;

    fn record_attempt(&mut self, attempt: ServiceAttemptRecord) -> Result<(), Self::Error> {
        self.assert_known(&attempt.review_unit_id)?;
        self.attempts.push(attempt);
        Ok(())
    }

    fn read_schedule_state(
        &self,
        review_unit_id: &ReviewUnitId,
    ) -> Result<Option<ScheduleState>, Self::Error> {
        self.assert_known(review_unit_id)?;
        Ok(self.schedules.get(review_unit_id).cloned())
    }

    fn apply_review(
        &mut self,
        review_unit_id: &ReviewUnitId,
        attempt: ServiceAttemptRecord,
        schedule_state: ScheduleState,
        _expected_prior_schedule_state: Option<ScheduleState>,
    ) -> Result<(), Self::Error> {
        self.assert_known(review_unit_id)?;
        self.attempts.push(attempt);
        self.schedules
            .insert(review_unit_id.clone(), schedule_state);
        Ok(())
    }

    fn list_queue_candidates(&self) -> Result<Vec<QueueCandidate>, Self::Error> {
        Ok(self
            .queue
            .values()
            .map(|candidate| {
                let schedule_state = self.schedules.get(&candidate.review_unit_id).cloned();

                QueueCandidate {
                    review_unit_id: candidate.review_unit_id.clone(),
                    due: schedule_state
                        .as_ref()
                        .map_or(candidate.due, |state| state.due),
                    schedule_state,
                    progression: candidate.progression.clone(),
                    concept_key: candidate.concept_key.clone(),
                    source_key: candidate.source_key.clone(),
                    domain_key: candidate.domain_key.clone(),
                }
            })
            .collect())
    }
}

#[must_use]
pub fn compile_latin_prayer_fixture(now: i64) -> CompiledImportProbe {
    compile_authored_fixture(&latin_prayer_fixture(), now)
}

/// Run the import probe through the Rust service boundary.
///
/// # Errors
///
/// Returns an error if fixture compilation or service execution fails.
pub fn run_import_probe() -> Result<ImportProbeReceipt, ImportProbeError> {
    let compiled = compile_latin_prayer_fixture(NOW);
    let first_prompt = compiled
        .prompts
        .first()
        .ok_or(ImportProbeError::EmptyFixture)?
        .clone();
    let first_review_unit_id = prompt_review_unit_id(&first_prompt).clone();
    let prompt_id = prompt_id_for(&compiled, &first_review_unit_id)?;
    let store = ImportProbeStore::new(&compiled);
    let mut service = MemoryService::with_clock(store, mastered_after_four_reviews, || NOW);
    let review = service.grade_apply_review(GradeApplyReviewCommand {
        prompt: first_prompt,
        submitted_answer: "I believe in one God".to_owned(),
        response_time_ms: 2_400,
        prompt_id: Some(prompt_id),
        occurred_at: Some(NOW),
        idempotency_key: None,
    })?;
    let next = service.next_queue(NextQueueCommand {
        options: NextQueueOptions::default(),
    })?;

    Ok(ImportProbeReceipt {
        fixture: compiled.fixture,
        prompts: compiled.prompts.len(),
        queue_candidates: compiled.queue.len(),
        schedules: compiled.schedules.len(),
        product_owned_fields: compiled.product_owned_fields,
        api_gap: compiled.api_gap,
        graded_verdict: verdict_name(review.grade.verdict).to_owned(),
        next_review_unit_id: next
            .candidate
            .as_ref()
            .map(|candidate| candidate.review_unit_id.as_str().to_owned()),
    })
}

fn compile_authored_fixture(fixture: &AuthoredFixture, now: i64) -> CompiledImportProbe {
    let mut prompts = Vec::new();
    let mut prompt_ids = Vec::new();
    let mut queue = Vec::new();
    let mut schedules = Vec::new();

    for card in &fixture.cards {
        assert!(
            !card.confidence_prompt.is_empty() && !card.notes.is_empty(),
            "authored import cards must carry product-owned study metadata"
        );
        let unit_id = ReviewUnitId::new(format!("import-{}", card.id));
        let prompt_id = format!("{}-translation", card.id);
        let schedule_state = if card.stage == ImportStage::Review {
            Some(review_schedule(now))
        } else {
            None
        };

        prompts.push(Prompt::Exact(ExactPrompt {
            kind: ExactPromptKind::ShortAnswer,
            review_unit_id: unit_id.clone(),
            prompt: format!("Translate: {}", card.source_text),
            accepted_answers: vec![card.translation.to_owned()],
            equivalence_groups: vec![vec!["God".to_owned(), "god".to_owned()]],
            ignored_tokens: PUNCTUATION_TOKENS
                .iter()
                .map(|token| (*token).to_owned())
                .collect(),
        }));
        prompt_ids.push((unit_id.clone(), prompt_id));
        queue.push(QueueCandidate {
            review_unit_id: unit_id.clone(),
            schedule_state: schedule_state.clone(),
            due: schedule_state
                .as_ref()
                .map_or(now - ONE_MINUTE_MS, |state| state.due),
            progression: None,
            concept_key: Some(card.concept_key.to_owned()),
            source_key: Some(fixture.source_key.to_owned()),
            domain_key: Some(fixture.domain_key.to_owned()),
        });

        if let Some(state) = schedule_state {
            schedules.push(CompiledSchedule {
                review_unit_id: unit_id,
                state,
            });
        }
    }

    CompiledImportProbe {
        fixture: fixture.id.to_owned(),
        prompts,
        prompt_ids,
        queue,
        schedules,
        product_owned_fields: vec![
            "sourceText".to_owned(),
            "translation".to_owned(),
            "confidencePrompt".to_owned(),
            "notes".to_owned(),
        ],
        api_gap: None,
    }
}

fn latin_prayer_fixture() -> AuthoredFixture {
    AuthoredFixture {
        id: "latin-prayer-authored-v1",
        domain_key: "latin",
        source_key: "mass-ordinary",
        cards: vec![
            AuthoredCard {
                id: "credo-in-unum-deum",
                source_text: "Credo in unum Deum",
                translation: "I believe in one God",
                concept_key: "creed-opening",
                stage: ImportStage::Review,
                confidence_prompt: "How sure are you before revealing the answer?",
                notes: vec!["Keep confidence outside the kernel until a client proves the need."],
            },
            AuthoredCard {
                id: "pater-noster",
                source_text: "Pater noster",
                translation: "Our Father",
                concept_key: "lords-prayer-opening",
                stage: ImportStage::New,
                confidence_prompt: "How sure are you before revealing the answer?",
                notes: vec!["Authored taxonomy stays product-owned."],
            },
        ],
    }
}

fn review_schedule(now: i64) -> ScheduleState {
    ScheduleState {
        due: now - ONE_MINUTE_MS,
        stability: 4.2,
        difficulty: 3.1,
        elapsed_days: 1,
        scheduled_days: 1,
        reps: 3,
        lapses: 0,
        state: ScheduleStatus::Review,
        last_review: Some(now - ONE_DAY_MS),
    }
}

fn prompt_id_for(
    compiled: &CompiledImportProbe,
    review_unit_id: &ReviewUnitId,
) -> Result<String, ImportProbeError> {
    compiled
        .prompt_ids
        .iter()
        .find_map(|(id, prompt_id)| (id == review_unit_id).then(|| prompt_id.clone()))
        .ok_or_else(|| ImportProbeError::MissingPromptId(review_unit_id.clone()))
}

fn prompt_review_unit_id(prompt: &Prompt) -> &ReviewUnitId {
    match prompt {
        Prompt::Mcq { review_unit_id, .. } | Prompt::Boolean { review_unit_id, .. } => {
            review_unit_id
        }
        Prompt::Exact(prompt) => &prompt.review_unit_id,
    }
}

fn mastered_after_four_reviews(schedule: &ScheduleState) -> bool {
    schedule.state == ScheduleStatus::Review && schedule.reps >= 4
}

fn verdict_name(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Correct => "correct",
        Verdict::Close => "close",
        Verdict::Wrong => "wrong",
        Verdict::Revealed => "revealed",
    }
}

#[cfg(test)]
mod tests {
    use memory_engine_core::{ExactPromptKind, Prompt, ScheduleStatus};

    use super::{compile_latin_prayer_fixture, run_import_probe};

    const NOW: i64 = 1_779_984_000_000;

    #[test]
    fn compiles_authored_cards_into_canonical_service_inputs() {
        let compiled = compile_latin_prayer_fixture(NOW);

        assert_eq!(compiled.fixture, "latin-prayer-authored-v1");
        assert_eq!(
            compiled.product_owned_fields,
            ["sourceText", "translation", "confidencePrompt", "notes"]
        );
        assert_eq!(compiled.api_gap, None);
        assert_eq!(compiled.prompts.len(), 2);
        assert_eq!(compiled.queue.len(), 2);
        assert_eq!(compiled.schedules.len(), 1);

        let Prompt::Exact(first_prompt) = &compiled.prompts[0] else {
            panic!("first import prompt should be exact");
        };
        assert_eq!(first_prompt.kind, ExactPromptKind::ShortAnswer);
        assert_eq!(
            first_prompt.review_unit_id.as_str(),
            "import-credo-in-unum-deum"
        );
        assert_eq!(first_prompt.prompt, "Translate: Credo in unum Deum");
        assert_eq!(first_prompt.accepted_answers, ["I believe in one God"]);
        assert_eq!(
            first_prompt.equivalence_groups,
            [["God".to_owned(), "god".to_owned()]]
        );
        assert_eq!(first_prompt.ignored_tokens, [".", ",", ";", ":"]);

        let first_queue = &compiled.queue[0];
        assert_eq!(
            first_queue.review_unit_id.as_str(),
            "import-credo-in-unum-deum"
        );
        assert_eq!(first_queue.concept_key.as_deref(), Some("creed-opening"));
        assert_eq!(first_queue.source_key.as_deref(), Some("mass-ordinary"));
        assert_eq!(first_queue.domain_key.as_deref(), Some("latin"));
        assert_eq!(first_queue.due, NOW - 60_000);

        let schedule = &compiled.schedules[0];
        assert_eq!(
            schedule.review_unit_id.as_str(),
            "import-credo-in-unum-deum"
        );
        assert_eq!(schedule.state.state, ScheduleStatus::Review);
        assert_eq!(schedule.state.reps, 3);
        assert_eq!(schedule.state.last_review, Some(NOW - 86_400_000));
    }

    #[test]
    fn runs_imported_material_through_the_service_loop() {
        let receipt = run_import_probe().expect("receipt");

        assert_eq!(receipt.fixture, "latin-prayer-authored-v1");
        assert_eq!(receipt.prompts, 2);
        assert_eq!(receipt.queue_candidates, 2);
        assert_eq!(receipt.schedules, 1);
        assert_eq!(receipt.api_gap, None);
        assert_eq!(receipt.graded_verdict, "correct");
        assert_eq!(
            receipt.next_review_unit_id.as_deref(),
            Some("import-pater-noster")
        );
        assert!(receipt
            .product_owned_fields
            .contains(&"confidencePrompt".to_owned()));
    }
}
