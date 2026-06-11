//! Postgres production persistence adapter for memory-engine.
//!
//! This crate owns the SQL boundary for account-scoped production study state.
//! It intentionally stays outside `memory-engine-core` and keeps HTTP, auth,
//! generation providers, and UI state out of the database adapter.

use std::{cell::RefCell, error::Error, fmt};

use memory_engine_core::{Prompt, QueueCandidate, ReviewUnitId, ScheduleState};
use memory_engine_generation::BetaGenerationStore;
use memory_engine_persistence::{
    AppliedReviewReceipt, ApproveGeneratedPromptDraftOptions, BetaReviewUnitRecord,
    BetaStoreSnapshot, GeneratedPromptDraft, GeneratedPromptValidationStatus, GenerationRun,
    ReferenceSpan, ScheduleRecord, SourceDocument,
};
use memory_engine_service::{MemoryServiceStore, ServiceAttemptRecord};
use memory_engine_study::BetaStudyStore;
use postgres::Client;

pub const MIGRATION_SQL: &str = r"
CREATE TABLE IF NOT EXISTS memory_engine_accounts (
    account_id TEXT PRIMARY KEY,
    created_at_ms BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS memory_engine_api_sessions (
    account_id TEXT PRIMARY KEY REFERENCES memory_engine_accounts(account_id) ON DELETE CASCADE,
    session_token TEXT NOT NULL,
    updated_at_ms BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS memory_engine_browser_sessions (
    session_id_hash TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES memory_engine_accounts(account_id) ON DELETE CASCADE,
    session_token TEXT NOT NULL,
    csrf_token_hash TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    expires_at_ms BIGINT NOT NULL,
    revoked_at_ms BIGINT
);

CREATE INDEX IF NOT EXISTS memory_engine_browser_sessions_account_idx
    ON memory_engine_browser_sessions(account_id, expires_at_ms);

CREATE TABLE IF NOT EXISTS memory_engine_auth_challenges (
    challenge_hash TEXT PRIMARY KEY,
    email_normalized TEXT NOT NULL,
    expires_at_ms BIGINT NOT NULL,
    consumed_at_ms BIGINT
);

CREATE TABLE IF NOT EXISTS memory_engine_source_documents (
    account_id TEXT NOT NULL REFERENCES memory_engine_accounts(account_id) ON DELETE CASCADE,
    source_document_id TEXT NOT NULL,
    document JSONB NOT NULL,
    created_at_ms BIGINT NOT NULL,
    PRIMARY KEY (account_id, source_document_id)
);

CREATE TABLE IF NOT EXISTS memory_engine_reference_spans (
    account_id TEXT NOT NULL REFERENCES memory_engine_accounts(account_id) ON DELETE CASCADE,
    reference_span_id TEXT NOT NULL,
    source_document_id TEXT NOT NULL,
    span JSONB NOT NULL,
    created_at_ms BIGINT NOT NULL,
    PRIMARY KEY (account_id, reference_span_id),
    FOREIGN KEY (account_id, source_document_id)
        REFERENCES memory_engine_source_documents(account_id, source_document_id)
        ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS memory_engine_generation_runs (
    account_id TEXT NOT NULL REFERENCES memory_engine_accounts(account_id) ON DELETE CASCADE,
    generation_run_id TEXT NOT NULL,
    run JSONB NOT NULL,
    started_at_ms BIGINT NOT NULL,
    PRIMARY KEY (account_id, generation_run_id)
);

CREATE TABLE IF NOT EXISTS memory_engine_generated_prompt_drafts (
    account_id TEXT NOT NULL REFERENCES memory_engine_accounts(account_id) ON DELETE CASCADE,
    draft_id TEXT NOT NULL,
    review_unit_id TEXT NOT NULL,
    draft JSONB NOT NULL,
    created_at_ms BIGINT NOT NULL,
    PRIMARY KEY (account_id, draft_id)
);

CREATE TABLE IF NOT EXISTS memory_engine_review_units (
    account_id TEXT NOT NULL REFERENCES memory_engine_accounts(account_id) ON DELETE CASCADE,
    review_unit_id TEXT NOT NULL,
    record JSONB NOT NULL,
    created_at_ms BIGINT NOT NULL,
    archived_at_ms BIGINT,
    PRIMARY KEY (account_id, review_unit_id)
);

CREATE TABLE IF NOT EXISTS memory_engine_schedules (
    account_id TEXT NOT NULL REFERENCES memory_engine_accounts(account_id) ON DELETE CASCADE,
    review_unit_id TEXT NOT NULL,
    state JSONB NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    PRIMARY KEY (account_id, review_unit_id),
    FOREIGN KEY (account_id, review_unit_id)
        REFERENCES memory_engine_review_units(account_id, review_unit_id)
        ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS memory_engine_attempts (
    account_id TEXT NOT NULL REFERENCES memory_engine_accounts(account_id) ON DELETE CASCADE,
    attempt_id BIGSERIAL PRIMARY KEY,
    review_unit_id TEXT NOT NULL,
    prompt_id TEXT,
    idempotency_key TEXT,
    attempt JSONB NOT NULL,
    occurred_at_ms BIGINT NOT NULL,
    FOREIGN KEY (account_id, review_unit_id)
        REFERENCES memory_engine_review_units(account_id, review_unit_id)
        ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS memory_engine_applied_reviews (
    account_id TEXT NOT NULL REFERENCES memory_engine_accounts(account_id) ON DELETE CASCADE,
    receipt_key TEXT NOT NULL,
    review_unit_id TEXT NOT NULL,
    attempt JSONB NOT NULL,
    expected_prior_schedule_state JSONB,
    schedule_state JSONB NOT NULL,
    applied_at_ms BIGINT NOT NULL,
    PRIMARY KEY (account_id, receipt_key),
    FOREIGN KEY (account_id, review_unit_id)
        REFERENCES memory_engine_review_units(account_id, review_unit_id)
        ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS memory_engine_attempts_account_review_idx
    ON memory_engine_attempts(account_id, review_unit_id, occurred_at_ms);
";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountScope {
    account_id: String,
}

impl AccountScope {
    /// Build an account scope for all subsequent store operations.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError::BlankAccountId`] when the account id is blank.
    pub fn new(account_id: impl Into<String>) -> Result<Self, PostgresStoreError> {
        let account_id = account_id.into();
        if account_id.trim().is_empty() {
            return Err(PostgresStoreError::BlankAccountId);
        }

        Ok(Self { account_id })
    }

    #[must_use]
    pub fn account_id(&self) -> &str {
        &self.account_id
    }
}

pub struct PostgresStudyStore {
    client: RefCell<Client>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrowserSession {
    pub account_id: String,
    pub session_token: String,
    pub csrf_token_hash: String,
    pub expires_at_ms: i64,
}

/// TLS connector for Postgres, with Mozilla's compiled-in roots.
///
/// Managed providers (Neon) require TLS; the previous `NoTls` connector
/// failed their handshake outright in production. Postgres negotiates TLS
/// per the URL's `sslmode` (default `prefer`), so this connector also serves
/// plaintext local and test databases by falling back when the server does
/// not offer TLS.
///
/// # Panics
///
/// Panics only if rustls rejects its own default protocol versions, which
/// would be a build-level misconfiguration.
fn tls_connector() -> tokio_postgres_rustls::MakeRustlsConnect {
    static CONFIG: std::sync::OnceLock<std::sync::Arc<rustls::ClientConfig>> =
        std::sync::OnceLock::new();
    let config = CONFIG.get_or_init(|| {
        let roots = rustls::RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        };
        let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
        std::sync::Arc::new(
            rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .expect("rustls default protocol versions")
                .with_root_certificates(roots)
                .with_no_client_auth(),
        )
    });

    tokio_postgres_rustls::MakeRustlsConnect::new(rustls::ClientConfig::clone(config))
}

/// Open a Postgres client, negotiating TLS per the URL's `sslmode`.
///
/// SCRAM channel binding is disabled: Neon's proxy advertises it but the
/// handshake fails with "server did not use channel binding"; rustls already
/// authenticates the server against the webpki roots, which is the property
/// channel binding would add.
///
/// # Errors
///
/// Returns the Postgres error when the URL is malformed or the connection
/// fails.
///
/// # Panics
///
/// Panics only if rustls rejects its own default protocol versions, which
/// would be a build-level misconfiguration.
pub fn connect_client(url: &str) -> Result<Client, postgres::Error> {
    let mut config: postgres::Config = url.parse()?;
    config.channel_binding(postgres::config::ChannelBinding::Disable);

    config.connect(tls_connector())
}

impl PostgresStudyStore {
    /// Connect to Postgres, negotiating TLS per the URL's `sslmode`.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when the connection fails.
    pub fn connect(url: &str) -> Result<Self, PostgresStoreError> {
        let client = connect_client(url)?;

        Ok(Self {
            client: RefCell::new(client),
        })
    }

    /// Run the production schema migration.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when Postgres rejects the migration.
    pub fn migrate(&mut self) -> Result<(), PostgresStoreError> {
        self.client.borrow_mut().batch_execute(MIGRATION_SQL)?;

        Ok(())
    }

    /// Check whether an account row exists without creating it.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when Postgres rejects the read.
    pub fn account_exists(&mut self, account_id: &str) -> Result<bool, PostgresStoreError> {
        let row = self.client.borrow_mut().query_opt(
            "SELECT 1 FROM memory_engine_accounts WHERE account_id = $1",
            &[&account_id],
        )?;

        Ok(row.is_some())
    }

    /// Check whether the supplied API session token is current without creating
    /// the account.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when Postgres rejects the read.
    pub fn api_session_matches(
        &mut self,
        account_id: &str,
        session_token: &str,
    ) -> Result<bool, PostgresStoreError> {
        let row = self.client.borrow_mut().query_opt(
            "SELECT 1 FROM memory_engine_api_sessions
             WHERE account_id = $1 AND session_token = $2",
            &[&account_id, &session_token],
        )?;

        Ok(row.is_some())
    }

    /// Save a browser cookie session for later server-side resolution.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when Postgres rejects the write.
    pub fn save_browser_session(
        &mut self,
        session_id_hash: &str,
        account_id: &str,
        session_token: &str,
        csrf_token_hash: &str,
        now_ms: i64,
        expires_at_ms: i64,
    ) -> Result<(), PostgresStoreError> {
        self.client.borrow_mut().execute(
            "INSERT INTO memory_engine_browser_sessions
                (session_id_hash, account_id, session_token, csrf_token_hash, created_at_ms, expires_at_ms, revoked_at_ms)
             VALUES ($1, $2, $3, $4, $5, $6, NULL)
             ON CONFLICT (session_id_hash) DO UPDATE
             SET account_id = EXCLUDED.account_id,
                 session_token = EXCLUDED.session_token,
                 csrf_token_hash = EXCLUDED.csrf_token_hash,
                 created_at_ms = EXCLUDED.created_at_ms,
                 expires_at_ms = EXCLUDED.expires_at_ms,
                 revoked_at_ms = NULL",
            &[
                &session_id_hash,
                &account_id,
                &session_token,
                &csrf_token_hash,
                &now_ms,
                &expires_at_ms,
            ],
        )?;

        Ok(())
    }

    /// Load a current, unrevoked browser session.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when Postgres rejects the read.
    pub fn browser_session(
        &mut self,
        session_id_hash: &str,
        now_ms: i64,
    ) -> Result<Option<BrowserSession>, PostgresStoreError> {
        let row = self.client.borrow_mut().query_opt(
            "SELECT account_id, session_token, csrf_token_hash, expires_at_ms
             FROM memory_engine_browser_sessions
             WHERE session_id_hash = $1
               AND revoked_at_ms IS NULL
               AND expires_at_ms > $2",
            &[&session_id_hash, &now_ms],
        )?;

        Ok(row.map(|row| BrowserSession {
            account_id: row.get(0),
            session_token: row.get(1),
            csrf_token_hash: row.get(2),
            expires_at_ms: row.get(3),
        }))
    }

    /// Save a single-use magic-link challenge.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when Postgres rejects the write.
    pub fn save_auth_challenge(
        &mut self,
        challenge_hash: &str,
        email_normalized: &str,
        expires_at_ms: i64,
    ) -> Result<(), PostgresStoreError> {
        self.client.borrow_mut().execute(
            "INSERT INTO memory_engine_auth_challenges
                (challenge_hash, email_normalized, expires_at_ms, consumed_at_ms)
             VALUES ($1, $2, $3, NULL)
             ON CONFLICT (challenge_hash) DO UPDATE
             SET email_normalized = EXCLUDED.email_normalized,
                 expires_at_ms = EXCLUDED.expires_at_ms,
                 consumed_at_ms = NULL",
            &[&challenge_hash, &email_normalized, &expires_at_ms],
        )?;

        Ok(())
    }

    /// Atomically consume a current magic-link challenge.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when Postgres rejects the update.
    pub fn consume_auth_challenge(
        &mut self,
        challenge_hash: &str,
        now_ms: i64,
    ) -> Result<Option<String>, PostgresStoreError> {
        let row = self.client.borrow_mut().query_opt(
            "UPDATE memory_engine_auth_challenges
             SET consumed_at_ms = $2
             WHERE challenge_hash = $1
               AND consumed_at_ms IS NULL
               AND expires_at_ms > $2
             RETURNING email_normalized",
            &[&challenge_hash, &now_ms],
        )?;

        Ok(row.map(|row| row.get(0)))
    }

    /// Scope all following operations to one already-authenticated account.
    pub fn for_account(&mut self, scope: AccountScope) -> AccountStudyStore<'_> {
        AccountStudyStore {
            client: &self.client,
            scope,
        }
    }
}

pub struct AccountStudyStore<'a> {
    client: &'a RefCell<Client>,
    scope: AccountScope,
}

impl AccountStudyStore<'_> {
    /// Read all scoped study state in the beta-store snapshot shape.
    ///
    /// This is the bridge surface the HTTP study session needs before the API can
    /// stop opening the file-backed beta store directly.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when Postgres reads or JSON decoding fail.
    pub fn snapshot(&self) -> Result<BetaStoreSnapshot, PostgresStoreError> {
        Ok(BetaStoreSnapshot {
            version: 1,
            source_documents: self.source_documents()?,
            reference_spans: self.reference_spans()?,
            generated_prompt_drafts: self.generated_prompt_drafts()?,
            review_units: self.review_units()?,
            schedules: self.schedule_records()?,
            attempts: self.attempts()?,
            generation_runs: self.generation_runs()?,
            applied_reviews: self.applied_reviews()?,
        })
    }

    /// Ensure the scoped account row exists.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when the insert fails.
    pub fn ensure_account(&mut self, created_at_ms: i64) -> Result<(), PostgresStoreError> {
        self.client.borrow_mut().execute(
            "INSERT INTO memory_engine_accounts (account_id, created_at_ms)
             VALUES ($1, $2)
             ON CONFLICT (account_id) DO NOTHING",
            &[&self.scope.account_id, &created_at_ms],
        )?;

        Ok(())
    }

    /// Persist the latest API session token for the scoped account.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when the session cannot be saved.
    pub fn save_api_session(
        &mut self,
        session_token: &str,
        updated_at_ms: i64,
    ) -> Result<(), PostgresStoreError> {
        self.client.borrow_mut().execute(
            "INSERT INTO memory_engine_api_sessions (account_id, session_token, updated_at_ms)
             VALUES ($1, $2, $3)
             ON CONFLICT (account_id) DO UPDATE
             SET session_token = EXCLUDED.session_token,
                 updated_at_ms = EXCLUDED.updated_at_ms",
            &[&self.scope.account_id, &session_token, &updated_at_ms],
        )?;

        Ok(())
    }

    /// Check whether the supplied API session token is current for this account.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when Postgres rejects the read.
    pub fn api_session_matches(&self, session_token: &str) -> Result<bool, PostgresStoreError> {
        let row = self.client.borrow_mut().query_opt(
            "SELECT 1 FROM memory_engine_api_sessions
             WHERE account_id = $1 AND session_token = $2",
            &[&self.scope.account_id, &session_token],
        )?;

        Ok(row.is_some())
    }

    /// Check whether a client review idempotency key already applied.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when Postgres rejects the read.
    pub fn applied_review_idempotency_key_exists(
        &self,
        idempotency_key: &str,
    ) -> Result<bool, PostgresStoreError> {
        let receipt_key = idempotency_receipt_key(idempotency_key);
        let row = self.client.borrow_mut().query_opt(
            "SELECT 1 FROM memory_engine_applied_reviews
             WHERE account_id = $1 AND receipt_key = $2",
            &[&self.scope.account_id, &receipt_key],
        )?;

        Ok(row.is_some())
    }

    /// Save or replace a review unit for the scoped account.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when serialization or persistence fails.
    pub fn save_review_unit(
        &mut self,
        review_unit: &BetaReviewUnitRecord,
    ) -> Result<(), PostgresStoreError> {
        let value = serde_json::to_value(review_unit)?;
        self.client.borrow_mut().execute(
            "INSERT INTO memory_engine_review_units
                (account_id, review_unit_id, record, created_at_ms, archived_at_ms)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (account_id, review_unit_id) DO UPDATE
             SET record = EXCLUDED.record,
                 created_at_ms = EXCLUDED.created_at_ms,
                 archived_at_ms = EXCLUDED.archived_at_ms",
            &[
                &self.scope.account_id,
                &review_unit.review_unit_id.as_str(),
                &value,
                &review_unit.created_at,
                &review_unit.archived_at,
            ],
        )?;

        Ok(())
    }

    /// Promote an accepted generated draft into a review unit.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when the draft is unknown, rejected, or has
    /// no saved generation run.
    pub fn approve_generated_prompt_draft(
        &mut self,
        draft_id: &str,
        options: ApproveGeneratedPromptDraftOptions,
    ) -> Result<BetaReviewUnitRecord, PostgresStoreError> {
        let ApproveGeneratedPromptDraftOptions {
            initial_schedule_state,
        } = options;
        let snapshot = self.snapshot()?;
        let draft = snapshot
            .generated_prompt_drafts
            .iter()
            .find(|draft| draft.id == draft_id)
            .cloned()
            .ok_or_else(|| PostgresStoreError::UnknownGeneratedPromptDraft(draft_id.to_owned()))?;
        if draft.validation.status != GeneratedPromptValidationStatus::Accepted {
            return Err(PostgresStoreError::RejectedGeneratedPromptDraft);
        }
        if draft
            .generation_run_id
            .as_ref()
            .is_none_or(|run_id| !snapshot.generation_runs.iter().any(|run| &run.id == run_id))
        {
            return Err(PostgresStoreError::MissingGenerationRunForAcceptedDraft);
        }

        let review_unit = BetaReviewUnitRecord {
            review_unit_id: draft.review_unit_id.clone(),
            prompt_id: draft.prompt_id.clone(),
            prompt: draft.prompt.clone(),
            queue: draft.queue.clone(),
            reference_span_ids: draft.reference_span_ids.clone(),
            generated_prompt_draft_id: Some(draft.id.clone()),
            archived_at: None,
            snoozed_until: None,
            created_at: draft.created_at,
        };
        self.save_review_unit(&review_unit)?;
        if let Some(schedule_state) = initial_schedule_state.as_ref() {
            self.set_schedule_state(
                &review_unit.review_unit_id,
                Some(schedule_state),
                draft.created_at,
            )?;
        }

        Ok(review_unit)
    }

    /// Replace review prompt text while preserving the same review unit.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when the review unit is unknown.
    pub fn update_review_unit_prompt_text(
        &mut self,
        review_unit_id: &ReviewUnitId,
        prompt_text: &str,
        expected_answer: &str,
    ) -> Result<BetaReviewUnitRecord, PostgresStoreError> {
        let mut review_unit = self.review_unit(review_unit_id)?;
        replace_prompt_text(&mut review_unit.prompt, prompt_text);
        replace_prompt_answer(&mut review_unit.prompt, expected_answer);
        self.save_review_unit(&review_unit)?;

        Ok(review_unit)
    }

    /// Hide a review unit from future queue selection.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when the review unit is unknown.
    pub fn archive_review_unit(
        &mut self,
        review_unit_id: &ReviewUnitId,
        archived_at: i64,
    ) -> Result<BetaReviewUnitRecord, PostgresStoreError> {
        let mut review_unit = self.review_unit(review_unit_id)?;
        review_unit.archived_at = Some(archived_at);
        self.save_review_unit(&review_unit)?;

        Ok(review_unit)
    }

    /// Move a review unit's beta queue availability without changing schedule history.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when the review unit is unknown.
    pub fn snooze_review_unit_until(
        &mut self,
        review_unit_id: &ReviewUnitId,
        snoozed_until: i64,
    ) -> Result<BetaReviewUnitRecord, PostgresStoreError> {
        let mut review_unit = self.review_unit(review_unit_id)?;
        review_unit.snoozed_until = Some(snoozed_until);
        self.save_review_unit(&review_unit)?;

        Ok(review_unit)
    }

    /// Save or replace source material for the scoped account.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when serialization or persistence fails.
    pub fn save_source_document(
        &mut self,
        document: &SourceDocument,
    ) -> Result<(), PostgresStoreError> {
        let value = serde_json::to_value(document)?;
        self.client.borrow_mut().execute(
            "INSERT INTO memory_engine_source_documents
                (account_id, source_document_id, document, created_at_ms)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (account_id, source_document_id) DO UPDATE
             SET document = EXCLUDED.document,
                 created_at_ms = EXCLUDED.created_at_ms",
            &[
                &self.scope.account_id,
                &document.id,
                &value,
                &document.created_at,
            ],
        )?;

        Ok(())
    }

    /// Save or replace a source reference span for the scoped account.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when serialization or persistence fails.
    pub fn save_reference_span(
        &mut self,
        reference: &ReferenceSpan,
    ) -> Result<(), PostgresStoreError> {
        let value = serde_json::to_value(reference)?;
        self.client.borrow_mut().execute(
            "INSERT INTO memory_engine_reference_spans
                (account_id, reference_span_id, source_document_id, span, created_at_ms)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (account_id, reference_span_id) DO UPDATE
             SET source_document_id = EXCLUDED.source_document_id,
                 span = EXCLUDED.span,
                 created_at_ms = EXCLUDED.created_at_ms",
            &[
                &self.scope.account_id,
                &reference.id,
                &reference.source_document_id,
                &value,
                &reference.created_at,
            ],
        )?;

        Ok(())
    }

    /// Save or replace a generation run receipt for the scoped account.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when serialization or persistence fails.
    pub fn save_generation_run(&mut self, run: &GenerationRun) -> Result<(), PostgresStoreError> {
        let value = serde_json::to_value(run)?;
        self.client.borrow_mut().execute(
            "INSERT INTO memory_engine_generation_runs
                (account_id, generation_run_id, run, started_at_ms)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (account_id, generation_run_id) DO UPDATE
             SET run = EXCLUDED.run,
                 started_at_ms = EXCLUDED.started_at_ms",
            &[&self.scope.account_id, &run.id, &value, &run.started_at],
        )?;

        Ok(())
    }

    /// Save or replace a generated prompt draft for the scoped account.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when serialization or persistence fails.
    pub fn save_generated_prompt_draft(
        &mut self,
        draft: &GeneratedPromptDraft,
    ) -> Result<(), PostgresStoreError> {
        let value = serde_json::to_value(draft)?;
        self.client.borrow_mut().execute(
            "INSERT INTO memory_engine_generated_prompt_drafts
                (account_id, draft_id, review_unit_id, draft, created_at_ms)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (account_id, draft_id) DO UPDATE
             SET review_unit_id = EXCLUDED.review_unit_id,
                 draft = EXCLUDED.draft,
                 created_at_ms = EXCLUDED.created_at_ms",
            &[
                &self.scope.account_id,
                &draft.id,
                &draft.review_unit_id.as_str(),
                &value,
                &draft.created_at,
            ],
        )?;

        Ok(())
    }

    /// Set or clear the schedule for a review unit in the scoped account.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when the review unit is unknown or the
    /// schedule write fails.
    pub fn set_schedule_state(
        &mut self,
        review_unit_id: &ReviewUnitId,
        schedule_state: Option<&ScheduleState>,
        updated_at_ms: i64,
    ) -> Result<(), PostgresStoreError> {
        self.assert_known_review_unit(review_unit_id)?;
        if let Some(schedule_state) = schedule_state {
            let value = serde_json::to_value(schedule_state)?;
            self.client.borrow_mut().execute(
                "INSERT INTO memory_engine_schedules
                    (account_id, review_unit_id, state, updated_at_ms)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT (account_id, review_unit_id) DO UPDATE
                 SET state = EXCLUDED.state,
                     updated_at_ms = EXCLUDED.updated_at_ms",
                &[
                    &self.scope.account_id,
                    &review_unit_id.as_str(),
                    &value,
                    &updated_at_ms,
                ],
            )?;
        } else {
            self.client.borrow_mut().execute(
                "DELETE FROM memory_engine_schedules
                 WHERE account_id = $1 AND review_unit_id = $2",
                &[&self.scope.account_id, &review_unit_id.as_str()],
            )?;
        }

        Ok(())
    }

    fn assert_known_review_unit(
        &mut self,
        review_unit_id: &ReviewUnitId,
    ) -> Result<(), PostgresStoreError> {
        let known = self.client.borrow_mut().query_opt(
            "SELECT 1 FROM memory_engine_review_units
             WHERE account_id = $1 AND review_unit_id = $2",
            &[&self.scope.account_id, &review_unit_id.as_str()],
        )?;
        if known.is_some() {
            Ok(())
        } else {
            Err(PostgresStoreError::UnknownReviewUnit(
                review_unit_id.clone(),
            ))
        }
    }

    fn review_unit(
        &self,
        review_unit_id: &ReviewUnitId,
    ) -> Result<BetaReviewUnitRecord, PostgresStoreError> {
        let row = self.client.borrow_mut().query_opt(
            "SELECT record FROM memory_engine_review_units
             WHERE account_id = $1 AND review_unit_id = $2",
            &[&self.scope.account_id, &review_unit_id.as_str()],
        )?;
        let Some(row) = row else {
            return Err(PostgresStoreError::UnknownReviewUnit(
                review_unit_id.clone(),
            ));
        };
        let value: serde_json::Value = row.get(0);

        Ok(serde_json::from_value(value)?)
    }

    fn source_documents(&self) -> Result<Vec<SourceDocument>, PostgresStoreError> {
        query_json_column(
            self.client,
            "SELECT document FROM memory_engine_source_documents
             WHERE account_id = $1
             ORDER BY created_at_ms, source_document_id",
            &self.scope.account_id,
        )
    }

    fn reference_spans(&self) -> Result<Vec<ReferenceSpan>, PostgresStoreError> {
        query_json_column(
            self.client,
            "SELECT span FROM memory_engine_reference_spans
             WHERE account_id = $1
             ORDER BY created_at_ms, reference_span_id",
            &self.scope.account_id,
        )
    }

    fn generated_prompt_drafts(&self) -> Result<Vec<GeneratedPromptDraft>, PostgresStoreError> {
        query_json_column(
            self.client,
            "SELECT draft FROM memory_engine_generated_prompt_drafts
             WHERE account_id = $1
             ORDER BY created_at_ms, draft_id",
            &self.scope.account_id,
        )
    }

    fn review_units(&self) -> Result<Vec<BetaReviewUnitRecord>, PostgresStoreError> {
        query_json_column(
            self.client,
            "SELECT record FROM memory_engine_review_units
             WHERE account_id = $1
             ORDER BY created_at_ms, review_unit_id",
            &self.scope.account_id,
        )
    }

    fn schedule_records(&self) -> Result<Vec<ScheduleRecord>, PostgresStoreError> {
        let rows = self.client.borrow_mut().query(
            "SELECT review_unit_id, state FROM memory_engine_schedules
             WHERE account_id = $1
             ORDER BY updated_at_ms, review_unit_id",
            &[&self.scope.account_id],
        )?;

        rows.into_iter()
            .map(|row| {
                let review_unit_id: String = row.get(0);
                let value: serde_json::Value = row.get(1);
                Ok(ScheduleRecord {
                    review_unit_id: ReviewUnitId::new(review_unit_id),
                    state: serde_json::from_value(value)?,
                })
            })
            .collect()
    }

    fn attempts(&self) -> Result<Vec<ServiceAttemptRecord>, PostgresStoreError> {
        query_json_column(
            self.client,
            "SELECT attempt FROM memory_engine_attempts
             WHERE account_id = $1
             ORDER BY occurred_at_ms, attempt_id",
            &self.scope.account_id,
        )
    }

    fn generation_runs(&self) -> Result<Vec<GenerationRun>, PostgresStoreError> {
        query_json_column(
            self.client,
            "SELECT run FROM memory_engine_generation_runs
             WHERE account_id = $1
             ORDER BY started_at_ms, generation_run_id",
            &self.scope.account_id,
        )
    }

    fn applied_reviews(&self) -> Result<Vec<AppliedReviewReceipt>, PostgresStoreError> {
        let rows = self.client.borrow_mut().query(
            "SELECT receipt_key, attempt, expected_prior_schedule_state, schedule_state
             FROM memory_engine_applied_reviews
             WHERE account_id = $1
             ORDER BY applied_at_ms, receipt_key",
            &[&self.scope.account_id],
        )?;

        rows.into_iter()
            .map(|row| {
                let key: String = row.get(0);
                let attempt: serde_json::Value = row.get(1);
                let expected_prior_schedule_state: Option<serde_json::Value> = row.get(2);
                let schedule_state: serde_json::Value = row.get(3);

                Ok(AppliedReviewReceipt {
                    key,
                    attempt: serde_json::from_value(attempt)?,
                    expected_prior_schedule_state: expected_prior_schedule_state
                        .map(serde_json::from_value)
                        .transpose()?,
                    schedule_state: serde_json::from_value(schedule_state)?,
                })
            })
            .collect()
    }
}

impl BetaGenerationStore for AccountStudyStore<'_> {
    type Error = PostgresStoreError;

    fn snapshot(&self) -> Result<BetaStoreSnapshot, Self::Error> {
        AccountStudyStore::snapshot(self)
    }

    fn save_generation_run(&mut self, run: GenerationRun) -> Result<GenerationRun, Self::Error> {
        AccountStudyStore::save_generation_run(self, &run)?;

        Ok(run)
    }

    fn save_reference_span(
        &mut self,
        reference: ReferenceSpan,
    ) -> Result<ReferenceSpan, Self::Error> {
        AccountStudyStore::save_reference_span(self, &reference)?;

        Ok(reference)
    }

    fn save_generated_prompt_draft(
        &mut self,
        draft: GeneratedPromptDraft,
    ) -> Result<GeneratedPromptDraft, Self::Error> {
        AccountStudyStore::save_generated_prompt_draft(self, &draft)?;

        Ok(draft)
    }
}

impl BetaStudyStore for AccountStudyStore<'_> {
    fn save_source_document(
        &mut self,
        document: SourceDocument,
    ) -> Result<SourceDocument, <Self as MemoryServiceStore>::Error> {
        AccountStudyStore::save_source_document(self, &document)?;

        Ok(document)
    }

    fn approve_generated_prompt_draft(
        &mut self,
        draft_id: &str,
        options: ApproveGeneratedPromptDraftOptions,
    ) -> Result<BetaReviewUnitRecord, <Self as MemoryServiceStore>::Error> {
        AccountStudyStore::approve_generated_prompt_draft(self, draft_id, options)
    }

    fn update_review_unit_prompt_text(
        &mut self,
        review_unit_id: &ReviewUnitId,
        prompt_text: &str,
        expected_answer: &str,
    ) -> Result<BetaReviewUnitRecord, <Self as MemoryServiceStore>::Error> {
        AccountStudyStore::update_review_unit_prompt_text(
            self,
            review_unit_id,
            prompt_text,
            expected_answer,
        )
    }

    fn archive_review_unit(
        &mut self,
        review_unit_id: &ReviewUnitId,
        archived_at: i64,
    ) -> Result<BetaReviewUnitRecord, <Self as MemoryServiceStore>::Error> {
        AccountStudyStore::archive_review_unit(self, review_unit_id, archived_at)
    }

    fn snooze_review_unit_until(
        &mut self,
        review_unit_id: &ReviewUnitId,
        snoozed_until: i64,
    ) -> Result<BetaReviewUnitRecord, <Self as MemoryServiceStore>::Error> {
        AccountStudyStore::snooze_review_unit_until(self, review_unit_id, snoozed_until)
    }
}

fn query_json_column<T>(
    client: &RefCell<Client>,
    sql: &str,
    account_id: &str,
) -> Result<Vec<T>, PostgresStoreError>
where
    T: serde::de::DeserializeOwned,
{
    let rows = client.borrow_mut().query(sql, &[&account_id])?;

    rows.into_iter()
        .map(|row| {
            let value: serde_json::Value = row.get(0);
            Ok(serde_json::from_value(value)?)
        })
        .collect()
}

impl MemoryServiceStore for AccountStudyStore<'_> {
    type Error = PostgresStoreError;

    fn record_attempt(&mut self, attempt: ServiceAttemptRecord) -> Result<(), Self::Error> {
        self.assert_known_review_unit(&attempt.review_unit_id)?;
        let value = serde_json::to_value(&attempt)?;
        self.client.borrow_mut().execute(
            "INSERT INTO memory_engine_attempts
                (account_id, review_unit_id, prompt_id, idempotency_key, attempt, occurred_at_ms)
             VALUES ($1, $2, $3, $4, $5, $6)",
            &[
                &self.scope.account_id,
                &attempt.review_unit_id.as_str(),
                &attempt.prompt_id,
                &attempt.idempotency_key,
                &value,
                &attempt.occurred_at,
            ],
        )?;

        Ok(())
    }

    fn read_schedule_state(
        &self,
        review_unit_id: &ReviewUnitId,
    ) -> Result<Option<ScheduleState>, Self::Error> {
        let row = self.client.borrow_mut().query_opt(
            "SELECT state FROM memory_engine_schedules
             WHERE account_id = $1 AND review_unit_id = $2",
            &[&self.scope.account_id, &review_unit_id.as_str()],
        )?;
        let Some(row) = row else {
            return Ok(None);
        };
        let value: serde_json::Value = row.get(0);

        Ok(Some(serde_json::from_value(value)?))
    }

    fn apply_review(
        &mut self,
        review_unit_id: &ReviewUnitId,
        attempt: ServiceAttemptRecord,
        schedule_state: ScheduleState,
        expected_prior_schedule_state: Option<ScheduleState>,
    ) -> Result<(), Self::Error> {
        if review_unit_id != &attempt.review_unit_id {
            return Err(PostgresStoreError::ReviewUnitMismatch);
        }
        if schedule_state.last_review != Some(attempt.occurred_at) {
            return Err(PostgresStoreError::ScheduleLastReviewMismatch);
        }
        let receipt_key = applied_review_key(&attempt);

        let mut client = self.client.borrow_mut();
        let mut transaction = client.transaction()?;
        let review_unit = transaction.query_opt(
            "SELECT 1 FROM memory_engine_review_units
             WHERE account_id = $1 AND review_unit_id = $2
             FOR UPDATE",
            &[&self.scope.account_id, &review_unit_id.as_str()],
        )?;
        if review_unit.is_none() {
            return Err(PostgresStoreError::UnknownReviewUnit(
                review_unit_id.clone(),
            ));
        }

        let existing = transaction.query_opt(
            "SELECT 1 FROM memory_engine_applied_reviews
             WHERE account_id = $1 AND receipt_key = $2",
            &[&self.scope.account_id, &receipt_key],
        )?;
        if existing.is_some() {
            return Err(PostgresStoreError::DuplicateAppliedReview(receipt_key));
        }

        let current_schedule = transaction.query_opt(
            "SELECT state FROM memory_engine_schedules
             WHERE account_id = $1 AND review_unit_id = $2",
            &[&self.scope.account_id, &review_unit_id.as_str()],
        )?;
        let current_schedule = current_schedule
            .map(|row| {
                let value: serde_json::Value = row.get(0);
                serde_json::from_value(value)
            })
            .transpose()?;
        if current_schedule != expected_prior_schedule_state {
            return Err(PostgresStoreError::StaleScheduleWrite(
                review_unit_id.clone(),
            ));
        }

        let attempt_value = serde_json::to_value(&attempt)?;
        let expected_value = expected_prior_schedule_state
            .as_ref()
            .map(serde_json::to_value)
            .transpose()?;
        let schedule_value = serde_json::to_value(&schedule_state)?;
        transaction.execute(
            "INSERT INTO memory_engine_attempts
                (account_id, review_unit_id, prompt_id, idempotency_key, attempt, occurred_at_ms)
             VALUES ($1, $2, $3, $4, $5, $6)",
            &[
                &self.scope.account_id,
                &review_unit_id.as_str(),
                &attempt.prompt_id,
                &attempt.idempotency_key,
                &attempt_value,
                &attempt.occurred_at,
            ],
        )?;
        transaction.execute(
            "INSERT INTO memory_engine_schedules
                (account_id, review_unit_id, state, updated_at_ms)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (account_id, review_unit_id) DO UPDATE
             SET state = EXCLUDED.state,
                 updated_at_ms = EXCLUDED.updated_at_ms",
            &[
                &self.scope.account_id,
                &review_unit_id.as_str(),
                &schedule_value,
                &attempt.occurred_at,
            ],
        )?;
        transaction.execute(
            "INSERT INTO memory_engine_applied_reviews
                (account_id, receipt_key, review_unit_id, attempt,
                 expected_prior_schedule_state, schedule_state, applied_at_ms)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            &[
                &self.scope.account_id,
                &receipt_key,
                &review_unit_id.as_str(),
                &attempt_value,
                &expected_value,
                &schedule_value,
                &attempt.occurred_at,
            ],
        )?;
        transaction.commit()?;

        Ok(())
    }

    fn list_queue_candidates(&self) -> Result<Vec<QueueCandidate>, Self::Error> {
        let rows = self.client.borrow_mut().query(
            "SELECT record FROM memory_engine_review_units
             WHERE account_id = $1 AND archived_at_ms IS NULL",
            &[&self.scope.account_id],
        )?;

        rows.into_iter()
            .map(|row| {
                let value: serde_json::Value = row.get(0);
                let record: BetaReviewUnitRecord = serde_json::from_value(value)?;
                let schedule_state = self.read_schedule_state(&record.review_unit_id)?;
                Ok(record.queue.with_schedule(schedule_state))
            })
            .collect()
    }
}

#[derive(Debug)]
pub enum PostgresStoreError {
    BlankAccountId,
    UnknownReviewUnit(ReviewUnitId),
    UnknownGeneratedPromptDraft(String),
    RejectedGeneratedPromptDraft,
    MissingGenerationRunForAcceptedDraft,
    ReviewUnitMismatch,
    ScheduleLastReviewMismatch,
    DuplicateAppliedReview(String),
    StaleScheduleWrite(ReviewUnitId),
    StudySession(String),
    Postgres(postgres::Error),
    Json(serde_json::Error),
}

impl fmt::Display for PostgresStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BlankAccountId => formatter.write_str("Account id must not be blank"),
            Self::UnknownReviewUnit(id) => write!(formatter, "Unknown review unit: {id}"),
            Self::UnknownGeneratedPromptDraft(id) => {
                write!(formatter, "Unknown generated prompt draft: {id}")
            }
            Self::RejectedGeneratedPromptDraft => {
                formatter.write_str("Generated prompt draft is not accepted")
            }
            Self::MissingGenerationRunForAcceptedDraft => {
                formatter.write_str("Accepted generated prompt draft requires a generation run")
            }
            Self::ReviewUnitMismatch => formatter.write_str("Review unit ids must match"),
            Self::ScheduleLastReviewMismatch => {
                formatter.write_str("Schedule last_review must match the attempt timestamp")
            }
            Self::DuplicateAppliedReview(key) => {
                write!(formatter, "Duplicate applied review: {key}")
            }
            Self::StaleScheduleWrite(id) => {
                write!(formatter, "Stale schedule write for review unit: {id}")
            }
            Self::StudySession(error) => write!(formatter, "Study session error: {error}"),
            Self::Postgres(error) => write!(formatter, "Postgres error: {error}"),
            Self::Json(error) => write!(formatter, "JSON error: {error}"),
        }
    }
}

impl Error for PostgresStoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Postgres(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::BlankAccountId
            | Self::UnknownReviewUnit(_)
            | Self::UnknownGeneratedPromptDraft(_)
            | Self::RejectedGeneratedPromptDraft
            | Self::MissingGenerationRunForAcceptedDraft
            | Self::ReviewUnitMismatch
            | Self::ScheduleLastReviewMismatch
            | Self::DuplicateAppliedReview(_)
            | Self::StaleScheduleWrite(_)
            | Self::StudySession(_) => None,
        }
    }
}

impl From<postgres::Error> for PostgresStoreError {
    fn from(error: postgres::Error) -> Self {
        Self::Postgres(error)
    }
}

impl From<serde_json::Error> for PostgresStoreError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<memory_engine_study::BetaStudyError<PostgresStoreError>> for PostgresStoreError {
    fn from(error: memory_engine_study::BetaStudyError<PostgresStoreError>) -> Self {
        match error {
            memory_engine_study::BetaStudyError::Store(error) => error,
            memory_engine_study::BetaStudyError::Generation(error) => {
                Self::StudySession(error.to_string())
            }
            memory_engine_study::BetaStudyError::Service(error) => {
                Self::StudySession(error.to_string())
            }
            memory_engine_study::BetaStudyError::NoActiveReviewUnit => {
                Self::StudySession(error.to_string())
            }
        }
    }
}

fn applied_review_key(attempt: &ServiceAttemptRecord) -> String {
    if let Some(idempotency_key) = &attempt.idempotency_key {
        return idempotency_receipt_key(idempotency_key);
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

fn idempotency_receipt_key(idempotency_key: &str) -> String {
    format!("idempotency:{idempotency_key}")
}

fn replace_prompt_text(prompt: &mut Prompt, text: &str) {
    match prompt {
        Prompt::Mcq { prompt, .. } | Prompt::Boolean { prompt, .. } => {
            text.clone_into(prompt);
        }
        Prompt::Exact(prompt) => {
            text.clone_into(&mut prompt.prompt);
        }
    }
}

fn replace_prompt_answer(prompt: &mut Prompt, answer: &str) {
    match prompt {
        Prompt::Mcq { correct_choice, .. } => {
            answer.clone_into(correct_choice);
        }
        Prompt::Boolean { correct_answer, .. } => {
            *correct_answer = matches!(
                answer.trim().to_ascii_lowercase().as_str(),
                "true" | "yes" | "1"
            );
        }
        Prompt::Exact(prompt) => {
            prompt.accepted_answers = vec![answer.to_owned()];
        }
    }
}

#[must_use]
pub fn migration_sql() -> &'static str {
    MIGRATION_SQL
}

#[must_use]
pub fn applied_review_receipt_key(attempt: &ServiceAttemptRecord) -> String {
    applied_review_key(attempt)
}

#[allow(dead_code)]
fn _receipt_type_anchor(_: &AppliedReviewReceipt) {}

#[cfg(test)]
mod tests {
    use memory_engine_core::{
        ExactPrompt, ExactPromptKind, Prompt, ReviewUnitId, ScheduleState, ScheduleStatus,
    };
    use memory_engine_persistence::{
        BetaReviewUnitRecord, BetaStoreSnapshot, GeneratedLearningActivityKind,
        GeneratedPromptDraft, GeneratedPromptModel, GeneratedPromptValidation,
        GeneratedPromptValidationStatus, GenerationRun, PersistedQueueCandidate, ReferenceSpan,
        SourceDocument, SourceDocumentKind, SourcePermission,
    };
    use memory_engine_service::ServiceAttemptRecord;
    use memory_engine_study::{BetaStudySession, BetaStudySourceInput, BetaStudyStatus};

    use super::{
        applied_review_receipt_key, migration_sql, AccountScope, MemoryServiceStore,
        PostgresStoreError, PostgresStudyStore,
    };

    const NOW: i64 = 1_779_465_600_000;

    #[test]
    fn account_scope_rejects_blank_ids() {
        assert!(AccountScope::new("acct_123").is_ok());
        assert!(AccountScope::new("  ").is_err());
    }

    #[test]
    fn migration_uses_account_scoped_primary_keys_and_durable_receipts() {
        let sql = migration_sql();

        assert!(sql.contains("memory_engine_accounts"));
        assert!(sql.contains("memory_engine_source_documents"));
        assert!(sql.contains("memory_engine_generation_runs"));
        assert!(sql.contains("memory_engine_generated_prompt_drafts"));
        assert!(sql.contains("PRIMARY KEY (account_id, review_unit_id)"));
        assert!(sql.contains("PRIMARY KEY (account_id, source_document_id)"));
        assert!(sql.contains("PRIMARY KEY (account_id, draft_id)"));
        assert!(sql.contains("PRIMARY KEY (account_id, receipt_key)"));
        assert!(sql.contains("memory_engine_applied_reviews"));
        assert!(sql.contains("memory_engine_browser_sessions"));
        assert!(sql.contains("session_id_hash TEXT PRIMARY KEY"));
        assert!(sql.contains("csrf_token_hash TEXT NOT NULL"));
        assert!(sql.contains("memory_engine_auth_challenges"));
        assert!(sql.contains("challenge_hash TEXT PRIMARY KEY"));
        assert!(sql.contains("consumed_at_ms BIGINT"));
        assert!(sql.contains("expected_prior_schedule_state JSONB"));
        assert!(sql.contains("ON DELETE CASCADE"));
    }

    #[test]
    fn applied_review_receipt_key_prefers_client_idempotency_key() {
        let attempt = ServiceAttemptRecord {
            review_unit_id: ReviewUnitId::new("unit-a"),
            prompt_id: Some("prompt-a".to_owned()),
            submitted_answer: "ALFA".to_owned(),
            response_time_ms: 1800,
            occurred_at: 1_779_465_600_000,
            idempotency_key: Some("mobile-submit-1".to_owned()),
            grade: None,
        };

        assert_eq!(
            applied_review_receipt_key(&attempt),
            "idempotency:mobile-submit-1"
        );
    }

    #[test]
    fn applied_review_receipt_key_falls_back_to_attempt_identity() {
        let attempt = ServiceAttemptRecord {
            review_unit_id: ReviewUnitId::new("unit-a"),
            prompt_id: Some("prompt-a".to_owned()),
            submitted_answer: "ALFA".to_owned(),
            response_time_ms: 1800,
            occurred_at: 1_779_465_600_000,
            idempotency_key: None,
            grade: None,
        };

        assert!(applied_review_receipt_key(&attempt).starts_with("attempt\0unit-a\0prompt-a"));
    }

    #[test]
    fn live_postgres_store_scopes_accounts_and_persists_idempotent_reviews() {
        let Some(database_url) = std::env::var("MEMORY_ENGINE_POSTGRES_TEST_URL").ok() else {
            eprintln!("skipping live Postgres test; MEMORY_ENGINE_POSTGRES_TEST_URL is unset");
            return;
        };
        let schema = format!("memory_engine_test_{}_{}", std::process::id(), NOW);
        let mut admin = crate::connect_client(&database_url).expect("connect admin postgres");
        admin
            .batch_execute(&format!(r#"CREATE SCHEMA "{schema}";"#))
            .expect("create schema");

        let scoped_url = format!(
            "{}{}options=-csearch_path%3D{}",
            database_url,
            if database_url.contains('?') { '&' } else { '?' },
            schema
        );
        let result = run_live_postgres_store_contract(&scoped_url);
        admin
            .batch_execute(&format!(r#"DROP SCHEMA "{schema}" CASCADE;"#))
            .expect("drop schema");
        result.expect("live postgres store contract");
    }

    fn run_live_postgres_store_contract(database_url: &str) -> Result<(), PostgresStoreError> {
        let mut store = PostgresStudyStore::connect(database_url)?;
        store.migrate()?;

        run_low_level_postgres_store_contract(&mut store)?;
        run_postgres_study_session_contract(&mut store)?;

        Ok(())
    }

    fn run_low_level_postgres_store_contract(
        store: &mut PostgresStudyStore,
    ) -> Result<(), PostgresStoreError> {
        let review_unit_id = ReviewUnitId::new("unit-live-a");
        let source = source_document("source-live-a");
        let reference = reference_span("reference-live-a", &source.id);
        let draft = accepted_draft(
            "draft-live-a",
            &review_unit_id,
            &[&source.id],
            &[&reference.id],
            Some("run-live-a"),
        );
        let run = generation_run("run-live-a", &[&source.id], &[&draft.id]);
        let review_unit = review_unit(&draft);
        let prior_schedule = schedule_state(1, ScheduleStatus::Review, NOW - 86_400_000);
        let next_schedule = schedule_state(2, ScheduleStatus::Review, NOW);
        let attempt = service_attempt(&review_unit_id, "idempotent-live-a", NOW);

        {
            let mut account_a = store.for_account(AccountScope::new("acct_live_a")?);
            account_a.ensure_account(NOW)?;
            account_a.save_source_document(&source)?;
            account_a.save_reference_span(&reference)?;
            account_a.save_generation_run(&run)?;
            account_a.save_generated_prompt_draft(&draft)?;
            account_a.save_review_unit(&review_unit)?;
            account_a.set_schedule_state(&review_unit_id, Some(&prior_schedule), NOW)?;

            assert_eq!(
                account_a.read_schedule_state(&review_unit_id)?,
                Some(prior_schedule.clone())
            );
            assert_eq!(account_a.list_queue_candidates()?.len(), 1);
            account_a.apply_review(
                &review_unit_id,
                attempt.clone(),
                next_schedule.clone(),
                Some(prior_schedule.clone()),
            )?;
            assert!(matches!(
                account_a.apply_review(
                    &review_unit_id,
                    attempt.clone(),
                    next_schedule.clone(),
                    Some(prior_schedule.clone())
                ),
                Err(PostgresStoreError::DuplicateAppliedReview(_))
            ));

            let snapshot = account_a.snapshot()?;
            assert_eq!(snapshot.source_documents, vec![source.clone()]);
            assert_eq!(snapshot.reference_spans, vec![reference.clone()]);
            assert_eq!(snapshot.generation_runs, vec![run.clone()]);
            assert_eq!(snapshot.generated_prompt_drafts, vec![draft.clone()]);
            assert_eq!(snapshot.review_units, vec![review_unit.clone()]);
            assert_eq!(snapshot.schedules.len(), 1);
            assert_eq!(snapshot.schedules[0].review_unit_id, review_unit_id);
            assert_eq!(snapshot.schedules[0].state, next_schedule.clone());
            assert_eq!(snapshot.attempts, vec![attempt.clone()]);
            assert_eq!(snapshot.applied_reviews.len(), 1);
            assert_eq!(
                snapshot.applied_reviews[0].key,
                "idempotency:idempotent-live-a"
            );
            assert_eq!(snapshot.applied_reviews[0].attempt, attempt);
            assert_eq!(
                snapshot.applied_reviews[0].expected_prior_schedule_state,
                Some(prior_schedule)
            );
            assert_eq!(snapshot.applied_reviews[0].schedule_state, next_schedule);
        }

        {
            let account_a = store.for_account(AccountScope::new("acct_live_a")?);
            assert_eq!(
                account_a.read_schedule_state(&review_unit_id)?,
                Some(next_schedule)
            );
        }

        {
            let mut account_b = store.for_account(AccountScope::new("acct_live_b")?);
            account_b.ensure_account(NOW)?;
            assert!(matches!(
                account_b.read_schedule_state(&review_unit_id),
                Ok(None)
            ));
            assert!(account_b.list_queue_candidates()?.is_empty());
            assert_eq!(account_b.snapshot()?, BetaStoreSnapshot::default());
        }

        Ok(())
    }

    fn run_postgres_study_session_contract(
        store: &mut PostgresStudyStore,
    ) -> Result<(), PostgresStoreError> {
        {
            let mut account = store.for_account(AccountScope::new("acct_live_study")?);
            account.ensure_account(NOW)?;
            let mut study = BetaStudySession::from_store(account, live_now);
            let sourced = study.add_source(study_source_input())?;
            assert_eq!(sourced.status, BetaStudyStatus::Drafting);
            assert_eq!(sourced.summary.source_count, 1);

            let generated = study.generate(None)?;
            assert_eq!(generated.drafts.len(), 2);
            assert_eq!(generated.summary.accepted_draft_count, 2);

            let approved =
                study.approve_draft("study-run-1-draft-src-nato-live-2-nato-cat-composition")?;
            assert_eq!(approved.status, BetaStudyStatus::Answering);
            assert_eq!(approved.summary.approved_review_unit_count, 1);

            let revealed = study.reveal()?;
            assert_eq!(
                revealed
                    .current
                    .as_ref()
                    .and_then(|current| current.expected_answer.as_deref()),
                Some("CHARLIE ALFA TANGO")
            );

            let reviewed = study.submit_answer("CHARLIE ALFA TANGO", 4_200)?;
            assert_eq!(reviewed.status, BetaStudyStatus::Graded);
            assert_eq!(reviewed.summary.attempt_count, 1);
            assert_eq!(
                reviewed
                    .current
                    .as_ref()
                    .and_then(|current| current.schedule_change.as_ref())
                    .and_then(|change| change.after.last_review),
                Some(NOW)
            );
        }

        {
            let account = store.for_account(AccountScope::new("acct_live_study")?);
            let snapshot = account.snapshot()?;
            assert_eq!(snapshot.source_documents.len(), 1);
            assert_eq!(snapshot.reference_spans.len(), 2);
            assert_eq!(snapshot.generation_runs.len(), 1);
            assert_eq!(snapshot.generated_prompt_drafts.len(), 2);
            assert_eq!(snapshot.review_units.len(), 1);
            assert_eq!(snapshot.attempts.len(), 1);
            assert_eq!(snapshot.applied_reviews.len(), 1);
        }

        Ok(())
    }

    fn source_document(id: &str) -> SourceDocument {
        SourceDocument {
            id: id.to_owned(),
            kind: SourceDocumentKind::Text,
            title: "Live Postgres source".to_owned(),
            body: Some("Pater noster means Our Father.".to_owned()),
            uri: None,
            permission: SourcePermission::ModelEligible,
            freshness: Some(NOW),
            created_at: NOW,
        }
    }

    fn study_source_input() -> BetaStudySourceInput {
        BetaStudySourceInput {
            id: "src-nato-live".to_owned(),
            title: "Live NATO practice notes".to_owned(),
            body: [
                "Concept: NATO letter A",
                "Activity: quiz",
                "Stage: recognition-3",
                "Question: What is the NATO phonetic alphabet word for A?",
                "Answer: ALFA",
                "Distractors: BRAVO, CHARLIE",
                "Reference: The NATO phonetic alphabet word for A is ALFA.",
                "",
                "Concept: NATO CAT composition",
                "Activity: exercise",
                "Stage: composition",
                "Question: Spell CAT over the phone using the NATO phonetic alphabet.",
                "Answer: CHARLIE ALFA TANGO",
                "Worked Solution: C is CHARLIE, A is ALFA, and T is TANGO.",
                "Reference: C is CHARLIE. A is ALFA. T is TANGO.",
            ]
            .join("\n"),
        }
    }

    fn live_now() -> i64 {
        NOW
    }

    fn reference_span(id: &str, source_document_id: &str) -> ReferenceSpan {
        ReferenceSpan {
            id: id.to_owned(),
            source_document_id: source_document_id.to_owned(),
            label: "line 1".to_owned(),
            text: "Pater noster means Our Father.".to_owned(),
            locator: "source:1".to_owned(),
            created_at: NOW,
        }
    }

    fn generation_run(id: &str, source_document_ids: &[&str], draft_ids: &[&str]) -> GenerationRun {
        GenerationRun {
            id: id.to_owned(),
            source_document_ids: source_document_ids
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            draft_ids: draft_ids.iter().map(|value| (*value).to_owned()).collect(),
            provider: "fixture".to_owned(),
            model: "deterministic-draft".to_owned(),
            started_at: NOW - 1_000,
            completed_at: Some(NOW),
            validation_failures: Vec::new(),
            usage: None,
        }
    }

    fn accepted_draft(
        id: &str,
        review_unit_id: &ReviewUnitId,
        source_document_ids: &[&str],
        reference_span_ids: &[&str],
        generation_run_id: Option<&str>,
    ) -> GeneratedPromptDraft {
        GeneratedPromptDraft {
            id: id.to_owned(),
            source_document_ids: source_document_ids
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            reference_span_ids: reference_span_ids
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            generation_run_id: generation_run_id.map(str::to_owned),
            review_unit_id: review_unit_id.clone(),
            prompt_id: "prompt-live-a".to_owned(),
            prompt: prompt(review_unit_id),
            queue: queue_candidate(review_unit_id),
            activity_kind: GeneratedLearningActivityKind::Quiz,
            activity_stage: "free-recall".to_owned(),
            worked_solution: None,
            model: GeneratedPromptModel {
                provider: "fixture".to_owned(),
                name: "deterministic-draft".to_owned(),
                version: "v1".to_owned(),
            },
            validation: GeneratedPromptValidation {
                status: GeneratedPromptValidationStatus::Accepted,
                reasons: Vec::new(),
            },
            critique_notes: vec!["Grounded in live Postgres source span.".to_owned()],
            created_at: NOW,
        }
    }

    fn review_unit(draft: &GeneratedPromptDraft) -> BetaReviewUnitRecord {
        BetaReviewUnitRecord {
            review_unit_id: draft.review_unit_id.clone(),
            prompt_id: draft.prompt_id.clone(),
            prompt: draft.prompt.clone(),
            queue: draft.queue.clone(),
            reference_span_ids: draft.reference_span_ids.clone(),
            generated_prompt_draft_id: Some(draft.id.clone()),
            archived_at: None,
            snoozed_until: None,
            created_at: NOW,
        }
    }

    fn prompt(review_unit_id: &ReviewUnitId) -> Prompt {
        Prompt::Exact(ExactPrompt {
            kind: ExactPromptKind::ShortAnswer,
            review_unit_id: review_unit_id.clone(),
            prompt: "Translate: Pater noster".to_owned(),
            accepted_answers: vec!["Our Father".to_owned()],
            equivalence_groups: Vec::new(),
            ignored_tokens: Vec::new(),
        })
    }

    fn queue_candidate(review_unit_id: &ReviewUnitId) -> PersistedQueueCandidate {
        PersistedQueueCandidate {
            review_unit_id: review_unit_id.clone(),
            due: NOW - 60_000,
            progression: None,
            concept_key: Some("latin-prayer-opening".to_owned()),
            source_key: Some("live-postgres-source".to_owned()),
            domain_key: Some("latin".to_owned()),
        }
    }

    fn schedule_state(reps: u32, status: ScheduleStatus, last_review: i64) -> ScheduleState {
        ScheduleState {
            due: NOW - 60_000,
            stability: 4.2,
            difficulty: 3.1,
            elapsed_days: 1,
            scheduled_days: 1,
            reps,
            lapses: 0,
            state: status,
            last_review: Some(last_review),
        }
    }

    fn service_attempt(
        review_unit_id: &ReviewUnitId,
        idempotency_key: &str,
        occurred_at: i64,
    ) -> ServiceAttemptRecord {
        ServiceAttemptRecord {
            review_unit_id: review_unit_id.clone(),
            prompt_id: Some("prompt-live-a".to_owned()),
            submitted_answer: "Our Father".to_owned(),
            response_time_ms: 1_800,
            occurred_at,
            idempotency_key: Some(idempotency_key.to_owned()),
            grade: None,
        }
    }
}
