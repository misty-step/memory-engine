//! Postgres production persistence adapter for memory-engine.
//!
//! This crate owns the SQL boundary for account-scoped production study state.
//! It intentionally stays outside `memory-engine-core` and keeps HTTP, auth,
//! generation providers, and UI state out of the database adapter.

use std::{
    cell::RefCell,
    collections::BTreeMap,
    error::Error,
    fmt,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, LazyLock,
    },
};

use memory_engine_core::{
    defer_queue_availability, Prompt, QueueCandidate, ReviewUnitId, ReviewUnitLifecycle,
    ScheduleState,
};
use memory_engine_generation::BetaGenerationStore;
use memory_engine_persistence::{
    parse_strict_boolean_answer, AppliedReviewReceipt, ApproveGeneratedPromptDraftOptions,
    BetaReviewUnitRecord, BetaStoreSnapshot, ConceptReferenceNote, GeneratedPromptDraft,
    GeneratedPromptValidationStatus, GenerationRun, ReferenceSpan, ScheduleRecord, SourceDocument,
};
use memory_engine_service::{
    content_feedback_replay_matches, ContentFeedback, ContentFeedbackStore, MemoryServiceStore,
    ServiceAttemptRecord,
};
use memory_engine_study::{select_current_review_unit, BetaStudyStore};
use postgres::types::ToSql;
use postgres::{Client, ToStatement};

static NEXT_LEASE_TOKEN: AtomicU64 = AtomicU64::new(1);

const RENEW_GENERATION_JOB_SQL: &str = "UPDATE memory_engine_generation_jobs
             SET lease_expires_at_ms = $4::BIGINT + $5::BIGINT, updated_at_ms = $4::BIGINT
             WHERE account_id = $1::TEXT AND job_id = $2::TEXT AND lease_token = $3::TEXT
               AND status = 'running' AND lease_expires_at_ms > $4::BIGINT";

const FINISH_GENERATION_JOB_SQL: &str = "UPDATE memory_engine_generation_jobs
             SET status = CASE
                     WHEN $1::BOOLEAN THEN 'succeeded'
                     WHEN attempts < $9::INTEGER THEN 'retry'
                     ELSE 'failed'
                 END,
                 card_count = $2::INTEGER,
                 cost_usd_micros = $3::BIGINT,
                 error = $4::TEXT,
                 retry_at_ms = CASE
                     WHEN $1::BOOLEAN THEN NULL::BIGINT
                     WHEN attempts < $9::INTEGER THEN $5::BIGINT
                     ELSE NULL::BIGINT
                 END,
                 lease_owner = NULL::TEXT,
                 lease_expires_at_ms = NULL::BIGINT,
                 lease_token = NULL::TEXT,
                 reserved_cost_usd_micros = CASE
                     WHEN $1::BOOLEAN THEN 0::BIGINT
                     WHEN attempts < $9::INTEGER THEN $11::BIGINT
                     ELSE 0::BIGINT
                 END,
                 updated_at_ms = $6::BIGINT
             WHERE account_id = $7::TEXT AND job_id = $8::TEXT AND lease_token = $10::TEXT
               AND status = 'running' AND lease_expires_at_ms > $6::BIGINT";

const FINISH_GENERATION_JOB_ATTEMPT_SQL: &str = "UPDATE memory_engine_generation_job_attempts
             SET status = $1::TEXT, cost_usd_micros = $2::BIGINT,
                 reserved_cost_usd_micros = 0::BIGINT, error = $3::TEXT,
                 completed_at_ms = $4::BIGINT, updated_at_ms = $4::BIGINT
             WHERE account_id = $5::TEXT AND job_id = $6::TEXT
               AND attempt = $7::INTEGER AND lease_token = $8::TEXT
               AND status = 'running'";

const BASE_MIGRATION_SQL: &str = r"
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

CREATE TABLE IF NOT EXISTS memory_engine_rate_limits (
    rate_limit_key TEXT PRIMARY KEY,
    window_start_ms BIGINT NOT NULL,
    attempt_count INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS memory_engine_auth_challenges (
    challenge_hash TEXT PRIMARY KEY,
    email_normalized TEXT NOT NULL,
    expires_at_ms BIGINT NOT NULL,
    consumed_at_ms BIGINT
);

CREATE TABLE IF NOT EXISTS memory_engine_return_notification_preferences (
    account_id TEXT PRIMARY KEY REFERENCES memory_engine_accounts(account_id) ON DELETE CASCADE,
    email_normalized TEXT NOT NULL,
    enabled BOOLEAN NOT NULL,
    last_sent_at_ms BIGINT,
    unsubscribe_nonce TEXT NOT NULL DEFAULT '',
    updated_at_ms BIGINT NOT NULL
);
ALTER TABLE memory_engine_return_notification_preferences
    ADD COLUMN IF NOT EXISTS claim_id TEXT,
    ADD COLUMN IF NOT EXISTS claim_expires_at_ms BIGINT,
    ADD COLUMN IF NOT EXISTS pending_delivery_key TEXT,
    ADD COLUMN IF NOT EXISTS pending_due_count BIGINT,
    ADD COLUMN IF NOT EXISTS pending_unsubscribe_expires_at_ms BIGINT,
    ADD COLUMN IF NOT EXISTS retry_attempts INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS next_retry_at_ms BIGINT,
    ADD COLUMN IF NOT EXISTS unsubscribe_nonce TEXT NOT NULL DEFAULT '';

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

CREATE TABLE IF NOT EXISTS memory_engine_concept_reference_notes (
    account_id TEXT NOT NULL REFERENCES memory_engine_accounts(account_id) ON DELETE CASCADE,
    concept_key TEXT NOT NULL,
    note JSONB NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    PRIMARY KEY (account_id, concept_key)
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

CREATE TABLE IF NOT EXISTS memory_engine_content_feedback (
    account_id TEXT NOT NULL REFERENCES memory_engine_accounts(account_id) ON DELETE CASCADE,
    feedback_id TEXT NOT NULL,
    review_unit_id TEXT NOT NULL,
    feedback JSONB NOT NULL,
    occurred_at_ms BIGINT NOT NULL,
    PRIMARY KEY (account_id, feedback_id),
    FOREIGN KEY (account_id, review_unit_id)
        REFERENCES memory_engine_review_units(account_id, review_unit_id)
        ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS memory_engine_content_feedback_account_review_idx
    ON memory_engine_content_feedback(account_id, review_unit_id, occurred_at_ms);
";

const CLAIM_RETURN_NOTIFICATION_SQL: &str = r"
UPDATE memory_engine_return_notification_preferences
 SET claim_id = $6,
     claim_expires_at_ms = $7::BIGINT,
     pending_delivery_key = COALESCE(pending_delivery_key, $8),
     pending_due_count = COALESCE(pending_due_count, $3::BIGINT),
    pending_unsubscribe_expires_at_ms = COALESCE(pending_unsubscribe_expires_at_ms, $10::BIGINT),
    retry_attempts = CASE
        WHEN memory_engine_return_notification_preferences.pending_delivery_key IS NOT NULL
        THEN memory_engine_return_notification_preferences.retry_attempts
        ELSE 0
    END,
    next_retry_at_ms = CASE
        WHEN memory_engine_return_notification_preferences.pending_delivery_key IS NOT NULL
        THEN memory_engine_return_notification_preferences.next_retry_at_ms
        ELSE NULL
    END,
     unsubscribe_nonce = COALESCE(NULLIF(unsubscribe_nonce, ''), $9),
     updated_at_ms = $2::BIGINT
 WHERE account_id = $1
   AND enabled
   AND ((pending_delivery_key IS NOT NULL AND
        (next_retry_at_ms IS NULL OR next_retry_at_ms <= $2::BIGINT)) OR
        (pending_delivery_key IS NULL AND ($4 OR $3::BIGINT > 0) AND
        (last_sent_at_ms IS NULL OR last_sent_at_ms <= $5::BIGINT)))
   AND (claim_expires_at_ms IS NULL OR claim_expires_at_ms <= $2::BIGINT)
 RETURNING email_normalized,
           pending_due_count,
           pending_delivery_key,
           unsubscribe_nonce,
           pending_unsubscribe_expires_at_ms
";

const GENERATION_JOBS_MIGRATION_SQL: &str = r"
CREATE TABLE IF NOT EXISTS memory_engine_generation_jobs (
    account_id TEXT NOT NULL REFERENCES memory_engine_accounts(account_id) ON DELETE CASCADE,
    job_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    title TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('queued', 'running', 'retry', 'succeeded', 'failed')),
    card_count INTEGER NOT NULL DEFAULT 0,
    attempts INTEGER NOT NULL DEFAULT 0,
    error TEXT,
    model_key TEXT NOT NULL,
    cost_usd_micros BIGINT NOT NULL DEFAULT 0,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    retry_at_ms BIGINT,
    lease_owner TEXT,
    lease_expires_at_ms BIGINT,
    lease_token TEXT,
    reserved_cost_usd_micros BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, job_id)
);

CREATE INDEX IF NOT EXISTS memory_engine_generation_jobs_queue_idx
    ON memory_engine_generation_jobs(status, retry_at_ms, created_at_ms);
CREATE INDEX IF NOT EXISTS memory_engine_generation_jobs_account_idx
    ON memory_engine_generation_jobs(account_id, created_at_ms DESC);
CREATE UNIQUE INDEX IF NOT EXISTS memory_engine_generation_jobs_active_source_idx
    ON memory_engine_generation_jobs(account_id, source_id)
    WHERE status IN ('queued', 'running', 'retry');
";

/// Additive generation-job changes for databases that already applied the v2
/// job ledger. `IF NOT EXISTS` makes this safe for both fresh and upgraded
/// installations; the version ledger applies it once during startup.
const GENERATION_JOBS_COMPATIBILITY_MIGRATION_SQL: &str = r"
ALTER TABLE memory_engine_generation_jobs
    ADD COLUMN IF NOT EXISTS lease_token TEXT,
    ADD COLUMN IF NOT EXISTS reserved_cost_usd_micros BIGINT NOT NULL DEFAULT 0;
";

const GENERATION_JOB_ATTEMPTS_MIGRATION_SQL: &str = r"
CREATE TABLE IF NOT EXISTS memory_engine_generation_job_attempts (
    account_id TEXT NOT NULL,
    job_id TEXT NOT NULL,
    attempt INTEGER NOT NULL,
    lease_token TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('running', 'succeeded', 'failed', 'stale')),
    generation_run_id TEXT,
    reservation_cost_usd_micros BIGINT NOT NULL DEFAULT 0,
    reserved_cost_usd_micros BIGINT NOT NULL DEFAULT 0,
    cost_usd_micros BIGINT NOT NULL DEFAULT 0,
    error TEXT,
    started_at_ms BIGINT NOT NULL,
    completed_at_ms BIGINT,
    updated_at_ms BIGINT NOT NULL,
    PRIMARY KEY (account_id, job_id, attempt),
    UNIQUE (account_id, job_id, lease_token),
    FOREIGN KEY (account_id, job_id)
        REFERENCES memory_engine_generation_jobs(account_id, job_id)
        ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS memory_engine_generation_job_attempts_budget_idx
    ON memory_engine_generation_job_attempts(account_id, started_at_ms);
ALTER TABLE memory_engine_generation_job_attempts
    ADD COLUMN IF NOT EXISTS generation_run_id TEXT;
";

/// The complete schema text remains available for smoke checks and operators.
/// Runtime migration application uses the versioned list below.
pub static MIGRATION_SQL: LazyLock<String> = LazyLock::new(|| {
    [
        BASE_MIGRATION_SQL,
        GENERATION_JOBS_MIGRATION_SQL,
        GENERATION_JOBS_COMPATIBILITY_MIGRATION_SQL,
        GENERATION_JOB_ATTEMPTS_MIGRATION_SQL,
    ]
    .concat()
});

static GENERATION_JOBS_MIGRATION_SQL_COMPLETE: LazyLock<String> = LazyLock::new(|| {
    [
        GENERATION_JOBS_MIGRATION_SQL,
        GENERATION_JOBS_COMPATIBILITY_MIGRATION_SQL,
        GENERATION_JOB_ATTEMPTS_MIGRATION_SQL,
    ]
    .concat()
});

const MIGRATION_TABLE_SQL: &str = r"
CREATE TABLE IF NOT EXISTS memory_engine_schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at_ms BIGINT NOT NULL
);
";

const MIGRATIONS: &[(i32, &str)] = &[
    (1, BASE_MIGRATION_SQL),
    (2, GENERATION_JOBS_MIGRATION_SQL),
    (3, GENERATION_JOBS_COMPATIBILITY_MIGRATION_SQL),
    (4, GENERATION_JOB_ATTEMPTS_MIGRATION_SQL),
];

fn migration_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(i64::MAX, |duration| {
            i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
        })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountScope {
    account_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PostgresGenerationJob {
    pub account_id: String,
    pub id: String,
    pub source_id: String,
    pub title: String,
    pub status: String,
    pub card_count: usize,
    pub attempts: u32,
    pub error: Option<String>,
    pub model_key: String,
    pub cost_usd_micros: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub retry_at: Option<i64>,
    pub lease_expires_at: Option<i64>,
    pub lease_token: Option<String>,
    pub reserved_cost_usd_micros: i64,
}

/// Durable accounting for one provider attempt.  The lease token is part of
/// the identity so a receipt can never be inferred from an unrelated run for
/// the same account and source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PostgresGenerationJobAttempt {
    pub account_id: String,
    pub job_id: String,
    pub attempt: u32,
    pub lease_token: String,
    pub status: String,
    pub generation_run_id: Option<String>,
    pub reservation_cost_usd_micros: i64,
    pub reserved_cost_usd_micros: i64,
    pub cost_usd_micros: i64,
    pub error: Option<String>,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PostgresEnqueueOutcome {
    Started(PostgresGenerationJob),
    AlreadyInFlight(PostgresGenerationJob),
    Rejected(String),
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

fn increment_statement_count(counter: &AtomicU64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
        Some(count.saturating_add(1))
    });
}

struct CountingClient {
    client: Client,
    statement_count: Arc<AtomicU64>,
}

impl CountingClient {
    fn new(client: Client) -> Self {
        Self {
            client,
            statement_count: Arc::new(AtomicU64::new(0)),
        }
    }

    fn statement_count(&self) -> u64 {
        self.statement_count.load(Ordering::Relaxed)
    }

    fn query<T>(
        &mut self,
        query: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<postgres::Row>, postgres::Error>
    where
        T: ?Sized + ToStatement,
    {
        increment_statement_count(&self.statement_count);
        self.client.query(query, params)
    }

    fn query_one<T>(
        &mut self,
        query: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<postgres::Row, postgres::Error>
    where
        T: ?Sized + ToStatement,
    {
        increment_statement_count(&self.statement_count);
        self.client.query_one(query, params)
    }

    fn query_opt<T>(
        &mut self,
        query: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<postgres::Row>, postgres::Error>
    where
        T: ?Sized + ToStatement,
    {
        increment_statement_count(&self.statement_count);
        self.client.query_opt(query, params)
    }

    fn execute<T>(
        &mut self,
        query: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<u64, postgres::Error>
    where
        T: ?Sized + ToStatement,
    {
        increment_statement_count(&self.statement_count);
        self.client.execute(query, params)
    }

    fn batch_execute(&mut self, query: &str) -> Result<(), postgres::Error> {
        increment_statement_count(&self.statement_count);
        self.client.batch_execute(query)
    }

    fn transaction(&mut self) -> Result<CountingTransaction<'_>, postgres::Error> {
        increment_statement_count(&self.statement_count);
        let transaction = self.client.transaction()?;
        Ok(CountingTransaction {
            transaction: Some(transaction),
            statement_count: Arc::clone(&self.statement_count),
        })
    }
}

struct CountingTransaction<'a> {
    transaction: Option<postgres::Transaction<'a>>,
    statement_count: Arc<AtomicU64>,
}

impl<'a> CountingTransaction<'a> {
    fn transaction(&mut self) -> &mut postgres::Transaction<'a> {
        self.transaction
            .as_mut()
            .expect("counted transaction remains live")
    }

    fn query<T>(
        &mut self,
        query: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<postgres::Row>, postgres::Error>
    where
        T: ?Sized + ToStatement,
    {
        increment_statement_count(&self.statement_count);
        self.transaction().query(query, params)
    }

    fn query_one<T>(
        &mut self,
        query: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<postgres::Row, postgres::Error>
    where
        T: ?Sized + ToStatement,
    {
        increment_statement_count(&self.statement_count);
        self.transaction().query_one(query, params)
    }

    fn query_opt<T>(
        &mut self,
        query: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<postgres::Row>, postgres::Error>
    where
        T: ?Sized + ToStatement,
    {
        increment_statement_count(&self.statement_count);
        self.transaction().query_opt(query, params)
    }

    fn execute<T>(
        &mut self,
        query: &T,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<u64, postgres::Error>
    where
        T: ?Sized + ToStatement,
    {
        increment_statement_count(&self.statement_count);
        self.transaction().execute(query, params)
    }

    fn batch_execute(&mut self, query: &str) -> Result<(), postgres::Error> {
        increment_statement_count(&self.statement_count);
        self.transaction().batch_execute(query)
    }

    fn commit(mut self) -> Result<(), postgres::Error> {
        increment_statement_count(&self.statement_count);
        self.transaction
            .take()
            .expect("counted transaction remains live")
            .commit()
    }

    fn rollback(mut self) -> Result<(), postgres::Error> {
        increment_statement_count(&self.statement_count);
        self.transaction
            .take()
            .expect("counted transaction remains live")
            .rollback()
    }
}

impl Drop for CountingTransaction<'_> {
    fn drop(&mut self) {
        if self.transaction.is_some() {
            increment_statement_count(&self.statement_count);
        }
    }
}

pub struct PostgresStudyStore {
    client: RefCell<CountingClient>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrowserSession {
    pub account_id: String,
    pub session_token: String,
    pub csrf_token_hash: String,
    pub expires_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReturnNotificationPreference {
    pub email: String,
    pub enabled: bool,
    pub last_sent_at_ms: Option<i64>,
    pub unsubscribe_nonce: String,
    pub claim_id: Option<String>,
    pub claim_expires_at_ms: Option<i64>,
    pub pending_delivery_key: Option<String>,
    pub pending_due_count: Option<i64>,
    pub pending_unsubscribe_expires_at_ms: Option<i64>,
    pub retry_attempts: i32,
    pub next_retry_at_ms: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnabledReturnNotificationAccount {
    pub account_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReturnNotificationClaim {
    pub email: String,
    pub due_count: i64,
    pub delivery_key: String,
    pub unsubscribe_nonce: String,
    pub unsubscribe_expires_at_ms: i64,
    pub claim_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReturnNotificationClaimRequest {
    pub account_id: String,
    pub now_ms: i64,
    pub due_count: i64,
    pub force_confirmation: bool,
    pub interval_ms: i64,
    pub claim_id: String,
    pub delivery_key: String,
    pub claim_expires_at_ms: i64,
    pub unsubscribe_nonce: String,
    pub unsubscribe_expires_at_ms: i64,
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
/// Connection establishment is retried with a short backoff: this store
/// opens a fresh connection per request, so without retry a single
/// transient (a proxy blip, a handshake reset) becomes a user-visible 500.
/// Seen live in dogfood: one "error performing TLS handshake" against a
/// warm Neon compute failed a login. Connecting is idempotent, so retry is
/// safe; queries are never retried here.
///
/// # Errors
///
/// Returns the Postgres error when the URL is malformed or every connection
/// attempt fails.
///
/// # Panics
///
/// Panics only if rustls rejects its own default protocol versions, which
/// would be a build-level misconfiguration.
pub fn connect_client(url: &str) -> Result<Client, postgres::Error> {
    let mut config: postgres::Config = url.parse()?;
    config.channel_binding(postgres::config::ChannelBinding::Disable);

    if url.contains("sslmode=disable") {
        retry_connect(|| config.connect(postgres::NoTls))
    } else {
        retry_connect(|| config.connect(tls_connector()))
    }
}

const CONNECT_ATTEMPTS: usize = 3;
const CONNECT_BACKOFF: std::time::Duration = std::time::Duration::from_millis(250);

fn retry_connect<T, E>(mut connect: impl FnMut() -> Result<T, E>) -> Result<T, E> {
    let mut attempt = 1;
    loop {
        match connect() {
            Ok(connection) => return Ok(connection),
            Err(error) => {
                if attempt >= CONNECT_ATTEMPTS {
                    return Err(error);
                }
                std::thread::sleep(CONNECT_BACKOFF * u32::try_from(attempt).unwrap_or(1));
                attempt += 1;
            }
        }
    }
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
            client: RefCell::new(CountingClient::new(client)),
        })
    }

    /// Return the cumulative number of database calls made by this store.
    #[must_use]
    pub fn statement_count(&self) -> u64 {
        self.client.borrow().statement_count()
    }

    /// Run the production schema migration.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when Postgres rejects the migration.
    pub fn migrate(&mut self) -> Result<(), PostgresStoreError> {
        let mut client = self.client.borrow_mut();
        client.batch_execute(MIGRATION_TABLE_SQL)?;
        let mut transaction = client.transaction()?;
        transaction.query_one("SELECT pg_advisory_xact_lock($1)", &[&9_301_094_i64])?;
        let applied =
            transaction.query("SELECT version FROM memory_engine_schema_migrations", &[])?;
        let applied = applied
            .into_iter()
            .map(|row| row.get::<_, i32>(0))
            .collect::<std::collections::BTreeSet<_>>();
        for (version, sql) in MIGRATIONS {
            if applied.contains(version) {
                continue;
            }
            transaction.batch_execute(sql)?;
            transaction.execute(
                "INSERT INTO memory_engine_schema_migrations (version, applied_at_ms)
                 VALUES ($1, $2)",
                &[version, &migration_now_ms()],
            )?;
        }
        transaction.commit()?;

        Ok(())
    }

    /// Check the database dependency without changing application state.
    ///
    /// # Errors
    /// Returns the Postgres error when the probe fails.
    pub fn ping(&mut self) -> Result<(), PostgresStoreError> {
        self.client.borrow_mut().query_one("SELECT 1", &[])?;
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

    /// Revoke a browser cookie session server-side.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when Postgres rejects the update.
    pub fn revoke_browser_session(
        &mut self,
        session_id_hash: &str,
        now_ms: i64,
    ) -> Result<(), PostgresStoreError> {
        self.client.borrow_mut().execute(
            "UPDATE memory_engine_browser_sessions
             SET revoked_at_ms = $2
             WHERE session_id_hash = $1",
            &[&session_id_hash, &now_ms],
        )?;

        Ok(())
    }

    /// Record one rate-limit attempt across every supplied key in a fixed
    /// window.
    ///
    /// Returns `true` when every key is still below the supplied limit and all
    /// increments were committed. Returns `false` without incrementing any key
    /// when one key is already exhausted.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when Postgres rejects the write.
    pub fn record_rate_limit_attempts(
        &mut self,
        keys: &[String],
        now_ms: i64,
        window_ms: i64,
        max_attempts: i32,
    ) -> Result<bool, PostgresStoreError> {
        let reset_before_ms = now_ms.saturating_sub(window_ms);
        let mut client = self.client.borrow_mut();
        let mut transaction = client.transaction()?;
        let mut sorted_keys = keys.iter().map(String::as_str).collect::<Vec<_>>();
        sorted_keys.sort_unstable();
        sorted_keys.dedup();
        let mut writes = Vec::with_capacity(sorted_keys.len());

        for key in sorted_keys {
            transaction.execute(
                "SELECT pg_advisory_xact_lock(
                    hashtext('memory_engine_rate_limits'),
                    hashtext($1)
                 )",
                &[&key],
            )?;
            let row = transaction.query_opt(
                "SELECT window_start_ms, attempt_count
                 FROM memory_engine_rate_limits
                 WHERE rate_limit_key = $1
                 FOR UPDATE",
                &[&key],
            )?;
            let (window_start_ms, attempt_count) = row.map_or((now_ms, 0), |row| {
                let window_start_ms: i64 = row.get(0);
                let attempt_count: i32 = row.get(1);
                if window_start_ms <= reset_before_ms {
                    (now_ms, 0)
                } else {
                    (window_start_ms, attempt_count)
                }
            });
            if attempt_count >= max_attempts {
                transaction.rollback()?;
                return Ok(false);
            }
            writes.push((key.to_owned(), window_start_ms, attempt_count + 1));
        }

        for (key, window_start_ms, attempt_count) in writes {
            transaction.execute(
                "INSERT INTO memory_engine_rate_limits
                    (rate_limit_key, window_start_ms, attempt_count)
                 VALUES ($1, $2, $3)
                 ON CONFLICT (rate_limit_key) DO UPDATE
                 SET window_start_ms = EXCLUDED.window_start_ms,
                     attempt_count = EXCLUDED.attempt_count",
                &[&key, &window_start_ms, &attempt_count],
            )?;
        }
        transaction.commit()?;

        Ok(true)
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

    /// Persist the learner's explicit due-count reminder choice.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when Postgres rejects the upsert.
    pub fn save_return_notification_preference(
        &mut self,
        account_id: &str,
        email_normalized: &str,
        enabled: bool,
        last_sent_at_ms: Option<i64>,
        updated_at_ms: i64,
        unsubscribe_nonce: &str,
    ) -> Result<(), PostgresStoreError> {
        self.client.borrow_mut().execute(
            "INSERT INTO memory_engine_return_notification_preferences
                (account_id, email_normalized, enabled, last_sent_at_ms, updated_at_ms, unsubscribe_nonce)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (account_id) DO UPDATE
             SET email_normalized = EXCLUDED.email_normalized,
                 enabled = EXCLUDED.enabled,
                 last_sent_at_ms = EXCLUDED.last_sent_at_ms,
                 unsubscribe_nonce = CASE
                     WHEN memory_engine_return_notification_preferences.enabled
                          AND EXCLUDED.enabled
                          AND memory_engine_return_notification_preferences.email_normalized = EXCLUDED.email_normalized
                          AND memory_engine_return_notification_preferences.pending_delivery_key IS NOT NULL
                     THEN memory_engine_return_notification_preferences.unsubscribe_nonce
                     ELSE EXCLUDED.unsubscribe_nonce
                 END,
                 claim_id = CASE
                     WHEN memory_engine_return_notification_preferences.enabled
                          AND EXCLUDED.enabled
                          AND memory_engine_return_notification_preferences.email_normalized = EXCLUDED.email_normalized
                          AND memory_engine_return_notification_preferences.pending_delivery_key IS NOT NULL
                     THEN memory_engine_return_notification_preferences.claim_id
                     ELSE NULL
                 END,
                 claim_expires_at_ms = CASE
                     WHEN memory_engine_return_notification_preferences.enabled
                          AND EXCLUDED.enabled
                          AND memory_engine_return_notification_preferences.email_normalized = EXCLUDED.email_normalized
                          AND memory_engine_return_notification_preferences.pending_delivery_key IS NOT NULL
                     THEN memory_engine_return_notification_preferences.claim_expires_at_ms
                     ELSE NULL
                 END,
                 pending_delivery_key = CASE
                     WHEN memory_engine_return_notification_preferences.enabled
                          AND EXCLUDED.enabled
                          AND memory_engine_return_notification_preferences.email_normalized = EXCLUDED.email_normalized
                          AND memory_engine_return_notification_preferences.pending_delivery_key IS NOT NULL
                     THEN memory_engine_return_notification_preferences.pending_delivery_key
                     ELSE NULL
                 END,
                 pending_due_count = CASE
                     WHEN memory_engine_return_notification_preferences.enabled
                          AND EXCLUDED.enabled
                          AND memory_engine_return_notification_preferences.email_normalized = EXCLUDED.email_normalized
                          AND memory_engine_return_notification_preferences.pending_delivery_key IS NOT NULL
                     THEN memory_engine_return_notification_preferences.pending_due_count
                     ELSE NULL
                 END,
                 pending_unsubscribe_expires_at_ms = CASE
                     WHEN memory_engine_return_notification_preferences.enabled
                          AND EXCLUDED.enabled
                          AND memory_engine_return_notification_preferences.email_normalized = EXCLUDED.email_normalized
                          AND memory_engine_return_notification_preferences.pending_delivery_key IS NOT NULL
                     THEN memory_engine_return_notification_preferences.pending_unsubscribe_expires_at_ms
                     ELSE NULL
                 END,
                 retry_attempts = CASE
                     WHEN memory_engine_return_notification_preferences.enabled
                          AND EXCLUDED.enabled
                          AND memory_engine_return_notification_preferences.email_normalized = EXCLUDED.email_normalized
                          AND memory_engine_return_notification_preferences.pending_delivery_key IS NOT NULL
                     THEN memory_engine_return_notification_preferences.retry_attempts
                     ELSE 0
                 END,
                 next_retry_at_ms = CASE
                     WHEN memory_engine_return_notification_preferences.enabled
                          AND EXCLUDED.enabled
                          AND memory_engine_return_notification_preferences.email_normalized = EXCLUDED.email_normalized
                          AND memory_engine_return_notification_preferences.pending_delivery_key IS NOT NULL
                     THEN memory_engine_return_notification_preferences.next_retry_at_ms
                     ELSE NULL
                 END,
                 updated_at_ms = EXCLUDED.updated_at_ms",
            &[
                &account_id,
                &email_normalized,
                &enabled,
                &last_sent_at_ms,
                &updated_at_ms,
                &unsubscribe_nonce,
            ],
        )?;
        Ok(())
    }

    /// Atomically fence one reminder delivery before the external mailer runs.
    /// The transaction is limited to this claim; callers must send mail after
    /// this method returns and finalize it with the claim id.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when the claim transaction cannot be
    /// committed or the database rejects the request.
    pub fn claim_return_notification(
        &mut self,
        request: &ReturnNotificationClaimRequest,
    ) -> Result<Option<ReturnNotificationClaim>, PostgresStoreError> {
        let threshold_ms = request.now_ms.saturating_sub(request.interval_ms);
        let mut client = self.client.borrow_mut();
        let mut transaction = client.transaction()?;
        let row = transaction.query_opt(
            CLAIM_RETURN_NOTIFICATION_SQL,
            &[
                &request.account_id,
                &request.now_ms,
                &request.due_count,
                &request.force_confirmation,
                &threshold_ms,
                &request.claim_id,
                &request.claim_expires_at_ms,
                &request.delivery_key,
                &request.unsubscribe_nonce,
                &request.unsubscribe_expires_at_ms,
            ],
        )?;
        transaction.commit()?;
        row.map(|row| {
            let due_count: i64 = row.get(1);
            let delivery_key: String = row.get(2);
            let unsubscribe_nonce: String = row.get(3);
            let unsubscribe_expires_at_ms: i64 = row.get(4);
            Ok(ReturnNotificationClaim {
                email: row.get(0),
                due_count,
                delivery_key,
                unsubscribe_nonce,
                unsubscribe_expires_at_ms,
                claim_id: request.claim_id.clone(),
            })
        })
        .transpose()
    }

    /// Enumerate enabled reminder accounts in a deterministic bounded batch.
    /// The scheduler still re-checks consent atomically when it claims each
    /// account, so an unsubscribe racing this read cannot send mail.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when Postgres rejects the query.
    pub fn enabled_return_notification_accounts(
        &mut self,
        limit: i64,
        now_ms: i64,
        interval_ms: i64,
    ) -> Result<Vec<EnabledReturnNotificationAccount>, PostgresStoreError> {
        let rows = self.client.borrow_mut().query(
            "SELECT account_id
             FROM memory_engine_return_notification_preferences
             WHERE enabled
               AND (claim_expires_at_ms IS NULL OR claim_expires_at_ms <= $2::BIGINT)
               AND ((pending_delivery_key IS NOT NULL
                     AND (next_retry_at_ms IS NULL OR next_retry_at_ms <= $2::BIGINT))
                    OR (pending_delivery_key IS NULL
                        AND (last_sent_at_ms IS NULL
                            OR last_sent_at_ms <= $2::BIGINT - $3::BIGINT)))
             ORDER BY account_id
             LIMIT $1",
            &[&limit, &now_ms, &interval_ms],
        )?;
        Ok(rows
            .into_iter()
            .map(|row| EnabledReturnNotificationAccount {
                account_id: row.get(0),
            })
            .collect())
    }

    /// Finalize only the currently fenced claim. A stale worker cannot mark a
    /// newer claim sent.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when Postgres rejects the update.
    pub fn complete_return_notification(
        &mut self,
        account_id: &str,
        claim_id: &str,
        sent_at_ms: i64,
    ) -> Result<bool, PostgresStoreError> {
        let changed = self.client.borrow_mut().execute(
            "UPDATE memory_engine_return_notification_preferences
             SET last_sent_at_ms = $3,
                 claim_id = NULL,
                 claim_expires_at_ms = NULL,
                 pending_delivery_key = NULL,
                 pending_due_count = NULL,
                 pending_unsubscribe_expires_at_ms = NULL,
                 retry_attempts = 0,
                 next_retry_at_ms = NULL,
                 updated_at_ms = $3
             WHERE account_id = $1 AND claim_id = $2",
            &[&account_id, &claim_id, &sent_at_ms],
        )?;
        Ok(changed == 1)
    }

    /// Release a failed claim while preserving its idempotency key for retry.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when Postgres rejects the update.
    pub fn release_return_notification(
        &mut self,
        account_id: &str,
        claim_id: &str,
        now_ms: i64,
    ) -> Result<(), PostgresStoreError> {
        self.client.borrow_mut().execute(
            "UPDATE memory_engine_return_notification_preferences
             SET claim_id = NULL,
                 claim_expires_at_ms = NULL,
                 retry_attempts = retry_attempts + 1,
                 next_retry_at_ms = $3::BIGINT + LEAST(
                     21600000::BIGINT,
                     60000::BIGINT * power(2::NUMERIC, LEAST(retry_attempts, 9))::BIGINT
                 )
             WHERE account_id = $1 AND claim_id = $2",
            &[&account_id, &claim_id, &now_ms],
        )?;
        Ok(())
    }

    /// Atomically consume a current unsubscribe token and rotate its nonce.
    ///
    /// The conditional update makes token consumption and nonce rotation one
    /// database operation, so a stale token cannot overwrite a concurrent
    /// authenticated re-enable.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when Postgres rejects the update.
    pub fn disable_return_notification(
        &mut self,
        account_id: &str,
        email_normalized: &str,
        current_nonce: &str,
        next_nonce: &str,
        updated_at_ms: i64,
    ) -> Result<bool, PostgresStoreError> {
        let changed = self.client.borrow_mut().execute(
            "UPDATE memory_engine_return_notification_preferences
             SET enabled = FALSE,
                 unsubscribe_nonce = $4,
                 claim_id = NULL,
                 claim_expires_at_ms = NULL,
                 pending_delivery_key = NULL,
                 pending_due_count = NULL,
                 pending_unsubscribe_expires_at_ms = NULL,
                 retry_attempts = 0,
                 next_retry_at_ms = NULL,
                 updated_at_ms = $5
             WHERE account_id = $1
               AND email_normalized = $2
               AND enabled
               AND unsubscribe_nonce = $3",
            &[
                &account_id,
                &email_normalized,
                &current_nonce,
                &next_nonce,
                &updated_at_ms,
            ],
        )?;
        Ok(changed == 1)
    }

    /// Load the learner's due-count reminder choice, if one exists.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when Postgres rejects the query.
    pub fn return_notification_preference(
        &mut self,
        account_id: &str,
    ) -> Result<Option<ReturnNotificationPreference>, PostgresStoreError> {
        let row = self.client.borrow_mut().query_opt(
            "SELECT email_normalized, enabled, last_sent_at_ms, unsubscribe_nonce,
                    claim_id, claim_expires_at_ms, pending_delivery_key, pending_due_count,
                    pending_unsubscribe_expires_at_ms, retry_attempts, next_retry_at_ms
             FROM memory_engine_return_notification_preferences
             WHERE account_id = $1",
            &[&account_id],
        )?;
        Ok(row.map(|row| ReturnNotificationPreference {
            email: row.get(0),
            enabled: row.get(1),
            last_sent_at_ms: row.get(2),
            unsubscribe_nonce: row.get(3),
            claim_id: row.get(4),
            claim_expires_at_ms: row.get(5),
            pending_delivery_key: row.get(6),
            pending_due_count: row.get(7),
            pending_unsubscribe_expires_at_ms: row.get(8),
            retry_attempts: row.get(9),
            next_retry_at_ms: row.get(10),
        }))
    }

    /// Scope all following operations to one already-authenticated account.
    pub fn for_account(&mut self, scope: AccountScope) -> AccountStudyStore<'_> {
        AccountStudyStore {
            client: &self.client,
            scope,
        }
    }

    /// Insert or coalesce a generation job under one transaction. Admission is
    /// database-owned so multiple API processes cannot exceed queue or budget
    /// limits and cannot duplicate one account/source.
    /// # Errors
    /// Returns the Postgres error when admission or insertion fails.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_lines)]
    pub fn enqueue_generation_job(
        &mut self,
        account_id: &str,
        job_id: &str,
        source_id: &str,
        title: &str,
        model_key: &str,
        now_ms: i64,
        max_account_queue: i64,
        max_global_queue: i64,
        max_account_model_cost_usd_micros: i64,
        budget_window_ms: i64,
    ) -> Result<PostgresEnqueueOutcome, PostgresStoreError> {
        let mut client = self.client.borrow_mut();
        let mut transaction = client.transaction()?;
        transaction.query_one(
            "SELECT pg_advisory_xact_lock($1::BIGINT)",
            &[&9_301_093_i64],
        )?;
        let existing = transaction.query_opt(
            "SELECT account_id, job_id, source_id, title, status, card_count,
                    attempts, error, model_key, cost_usd_micros, created_at_ms,
                    updated_at_ms, retry_at_ms, lease_expires_at_ms, lease_token,
                    reserved_cost_usd_micros
             FROM memory_engine_generation_jobs
             WHERE account_id = $1::TEXT AND source_id = $2::TEXT
               AND status IN ('queued', 'running', 'retry')
             ORDER BY created_at_ms LIMIT 1 FOR UPDATE",
            &[&account_id, &source_id],
        )?;
        if let Some(row) = existing {
            let job = generation_job_from_row(&row);
            transaction.commit()?;
            return Ok(PostgresEnqueueOutcome::AlreadyInFlight(job));
        }

        let account_queue: i64 = transaction
            .query_one(
                "SELECT COUNT(*) FROM memory_engine_generation_jobs
                 WHERE account_id = $1::TEXT AND status IN ('queued', 'running', 'retry')",
                &[&account_id],
            )?
            .get(0);
        if account_queue >= max_account_queue {
            transaction.rollback()?;
            return Ok(PostgresEnqueueOutcome::Rejected(
                "Generation queue is full for this account. Try again after current work finishes."
                    .to_owned(),
            ));
        }
        let global_queue: i64 = transaction
            .query_one(
                "SELECT COUNT(*) FROM memory_engine_generation_jobs
                 WHERE status IN ('queued', 'running', 'retry')",
                &[],
            )?
            .get(0);
        if global_queue >= max_global_queue {
            transaction.rollback()?;
            return Ok(PostgresEnqueueOutcome::Rejected(
                "Generation queue is full. Try again after current work finishes.".to_owned(),
            ));
        }
        // The configured queue share is the per-attempt maximum. Admission
        // below counts this proposed reservation, so concurrent jobs cannot
        // collectively promise more than the account/model budget.
        let reservation = (max_account_model_cost_usd_micros / max_account_queue.max(1)).max(1);
        let spent: i64 = transaction
            .query_one(
                "SELECT (
                    SELECT COALESCE(SUM(attempt.cost_usd_micros + attempt.reserved_cost_usd_micros), 0)::BIGINT
                    FROM memory_engine_generation_job_attempts attempt
                    JOIN memory_engine_generation_jobs job
                      ON job.account_id = attempt.account_id AND job.job_id = attempt.job_id
                    WHERE job.account_id = $1::TEXT AND job.model_key = $2::TEXT
                      AND attempt.started_at_ms >= $3::BIGINT
                ) + (
                    SELECT COALESCE(SUM(
                        CASE
                            WHEN EXISTS (
                                SELECT 1
                                FROM memory_engine_generation_job_attempts attempt
                                WHERE attempt.account_id = job.account_id
                                  AND attempt.job_id = job.job_id
                            )
                            THEN CASE WHEN job.status IN ('queued', 'retry')
                                      THEN job.reserved_cost_usd_micros ELSE 0 END
                            ELSE job.cost_usd_micros +
                                 CASE WHEN job.status IN ('queued', 'running', 'retry')
                                      THEN job.reserved_cost_usd_micros ELSE 0 END
                        END
                    ), 0)::BIGINT
                    FROM memory_engine_generation_jobs job
                    WHERE job.account_id = $1::TEXT AND job.model_key = $2::TEXT
                      AND job.created_at_ms >= $3::BIGINT
                )::BIGINT",
                &[
                    &account_id,
                    &model_key,
                    &now_ms.saturating_sub(budget_window_ms),
                ],
            )?
            .get(0);
        if spent.saturating_add(reservation) > max_account_model_cost_usd_micros {
            transaction.rollback()?;
            return Ok(PostgresEnqueueOutcome::Rejected(format!(
                "Generation budget for model {model_key} is exhausted for this account."
            )));
        }

        transaction.execute(
            "INSERT INTO memory_engine_generation_jobs
                (account_id, job_id, source_id, title, status, card_count, attempts,
                 error, model_key, cost_usd_micros, reserved_cost_usd_micros,
                 created_at_ms, updated_at_ms)
             VALUES ($1::TEXT, $2::TEXT, $3::TEXT, $4::TEXT, 'queued', 0, 0,
                     NULL::TEXT, $5::TEXT, 0::BIGINT, $7::BIGINT, $6::BIGINT, $6::BIGINT)",
            &[
                &account_id,
                &job_id,
                &source_id,
                &title,
                &model_key,
                &now_ms,
                &reservation,
            ],
        )?;
        transaction.commit()?;
        Ok(PostgresEnqueueOutcome::Started(PostgresGenerationJob {
            account_id: account_id.to_owned(),
            id: job_id.to_owned(),
            source_id: source_id.to_owned(),
            title: title.to_owned(),
            status: "queued".to_owned(),
            card_count: 0,
            attempts: 0,
            error: None,
            model_key: model_key.to_owned(),
            cost_usd_micros: 0,
            created_at: now_ms,
            updated_at: now_ms,
            retry_at: None,
            lease_expires_at: None,
            lease_token: None,
            reserved_cost_usd_micros: reservation,
        }))
    }

    /// # Errors
    /// Returns the Postgres error when the job history cannot be read.
    pub fn list_generation_jobs(
        &mut self,
        account_id: &str,
        limit: i64,
    ) -> Result<Vec<PostgresGenerationJob>, PostgresStoreError> {
        let rows = self.client.borrow_mut().query(
            "SELECT account_id, job_id, source_id, title, status, card_count,
                    attempts, error, model_key, cost_usd_micros, created_at_ms,
                    updated_at_ms, retry_at_ms, lease_expires_at_ms, lease_token,
                    reserved_cost_usd_micros
             FROM memory_engine_generation_jobs
             WHERE account_id = $1::TEXT ORDER BY created_at_ms DESC LIMIT $2::BIGINT",
            &[&account_id, &limit],
        )?;
        Ok(rows.iter().map(generation_job_from_row).collect())
    }

    /// # Errors
    /// Returns the Postgres error when the job cannot be read.
    pub fn generation_job(
        &mut self,
        account_id: &str,
        job_id: &str,
    ) -> Result<Option<PostgresGenerationJob>, PostgresStoreError> {
        let row = self.client.borrow_mut().query_opt(
            "SELECT account_id, job_id, source_id, title, status, card_count,
                    attempts, error, model_key, cost_usd_micros, created_at_ms,
                    updated_at_ms, retry_at_ms, lease_expires_at_ms, lease_token,
                    reserved_cost_usd_micros
             FROM memory_engine_generation_jobs
             WHERE account_id = $1::TEXT AND job_id = $2::TEXT
             LIMIT 1",
            &[&account_id, &job_id],
        )?;
        Ok(row.as_ref().map(generation_job_from_row))
    }

    /// Extend one claimed lease without changing its fencing token. A worker
    /// that lost the claim receives `false` and must stop before completion.
    ///
    /// # Errors
    /// Returns the Postgres error when the lease update fails.
    pub fn renew_generation_job(
        &mut self,
        account_id: &str,
        job_id: &str,
        lease_token: &str,
        now_ms: i64,
        lease_ms: i64,
    ) -> Result<bool, PostgresStoreError> {
        let changed = self.client.borrow_mut().execute(
            RENEW_GENERATION_JOB_SQL,
            &[&account_id, &job_id, &lease_token, &now_ms, &lease_ms],
        )?;
        Ok(changed == 1)
    }

    /// # Errors
    /// Returns the Postgres error when usage receipts cannot be read.
    pub fn generation_cost_for_source(
        &mut self,
        account_id: &str,
        source_id: &str,
    ) -> Result<i64, PostgresStoreError> {
        let row = self.client.borrow_mut().query_one(
            "SELECT COALESCE((run->'usage'->>'costUsdMicros')::BIGINT, 0)::BIGINT
             FROM memory_engine_generation_runs
             WHERE account_id = $1::TEXT
               AND run->'sourceDocumentIds' @> jsonb_build_array($2::TEXT)
               AND (run->>'completedAt') IS NOT NULL
             ORDER BY started_at_ms DESC, generation_run_id DESC
             LIMIT 1",
            &[&account_id, &source_id],
        )?;
        Ok(row.get(0))
    }

    /// Read usage for the exact run owned by one generation-job attempt.
    /// Unlike the legacy source lookup this cannot misattribute a concurrent
    /// direct generation or another job's run.
    ///
    /// # Errors
    /// Returns the Postgres error when the run receipt cannot be read.
    pub fn generation_cost_for_run(
        &mut self,
        account_id: &str,
        run_id: &str,
    ) -> Result<i64, PostgresStoreError> {
        let row = self.client.borrow_mut().query_one(
            "SELECT COALESCE((run->'usage'->>'costUsdMicros')::BIGINT, 0)::BIGINT
             FROM memory_engine_generation_runs
             WHERE account_id = $1::TEXT AND generation_run_id = $2::TEXT
             LIMIT 1",
            &[&account_id, &run_id],
        )?;
        Ok(row.get(0))
    }

    /// Check whether a durable generation attempt still owns the exact lease
    /// token and remains unexpired at the commit boundary.
    ///
    /// # Errors
    /// Returns the Postgres error when the receipt cannot be read.
    pub fn generation_job_attempt_has_commit_fence(
        &mut self,
        account_id: &str,
        run_id: &str,
        attempt: i32,
        lease_token: &str,
        now_ms: i64,
    ) -> Result<bool, PostgresStoreError> {
        let row = self.client.borrow_mut().query_opt(
            "SELECT 1
             FROM memory_engine_generation_job_attempts attempt
             JOIN memory_engine_generation_jobs job
               ON job.account_id = attempt.account_id AND job.job_id = attempt.job_id
             WHERE attempt.account_id = $1::TEXT
               AND attempt.generation_run_id = $2::TEXT
               AND attempt.attempt = $3::INTEGER
               AND attempt.lease_token = $4::TEXT
               AND attempt.status = 'running'
               AND job.status = 'running'
               AND job.attempts = attempt.attempt
               AND job.lease_token = attempt.lease_token
               AND job.lease_expires_at_ms > $5::BIGINT
             FOR UPDATE OF attempt, job
             LIMIT 1",
            &[&account_id, &run_id, &attempt, &lease_token, &now_ms],
        )?;
        Ok(row.is_some())
    }

    /// Read the receipt for one exact durable provider attempt.
    ///
    /// # Errors
    /// Returns the Postgres error when the receipt cannot be read.
    pub fn generation_job_attempt(
        &mut self,
        account_id: &str,
        job_id: &str,
        attempt: u32,
        lease_token: &str,
    ) -> Result<Option<PostgresGenerationJobAttempt>, PostgresStoreError> {
        let attempt = i32::try_from(attempt).unwrap_or(i32::MAX);
        let row = self.client.borrow_mut().query_opt(
            "SELECT account_id, job_id, attempt, lease_token, status, generation_run_id,
                    reservation_cost_usd_micros, reserved_cost_usd_micros,
                    cost_usd_micros, error, started_at_ms, completed_at_ms,
                    updated_at_ms
             FROM memory_engine_generation_job_attempts
             WHERE account_id = $1::TEXT AND job_id = $2::TEXT
               AND attempt = $3::INTEGER AND lease_token = $4::TEXT",
            &[&account_id, &job_id, &attempt, &lease_token],
        )?;
        Ok(row.map(|row| PostgresGenerationJobAttempt {
            account_id: row.get(0),
            job_id: row.get(1),
            attempt: u32::try_from(row.get::<_, i32>(2)).unwrap_or(0),
            lease_token: row.get(3),
            status: row.get(4),
            generation_run_id: row.get(5),
            reservation_cost_usd_micros: row.get(6),
            reserved_cost_usd_micros: row.get(7),
            cost_usd_micros: row.get(8),
            error: row.get(9),
            started_at: row.get(10),
            completed_at: row.get(11),
            updated_at: row.get(12),
        }))
    }

    /// Bind the provider's durable run identity before external work starts.
    /// This lets crash recovery reconcile a completed provider run even when
    /// the worker dies before it can finish the job row.
    ///
    /// # Errors
    /// Returns the Postgres error when the attempt receipt cannot be updated.
    pub fn bind_generation_job_attempt_run(
        &mut self,
        account_id: &str,
        job_id: &str,
        attempt: u32,
        lease_token: &str,
        run_id: &str,
    ) -> Result<bool, PostgresStoreError> {
        let attempt = i32::try_from(attempt).unwrap_or(i32::MAX);
        let changed = self.client.borrow_mut().execute(
            "UPDATE memory_engine_generation_job_attempts
             SET generation_run_id = $5::TEXT
             WHERE account_id = $1::TEXT AND job_id = $2::TEXT
               AND attempt = $3::INTEGER AND lease_token = $4::TEXT
               AND status = 'running'",
            &[&account_id, &job_id, &attempt, &lease_token, &run_id],
        )?;
        Ok(changed == 1)
    }

    /// Claim one job with a lease. The advisory transaction lock makes the
    /// global and per-account concurrency checks correct across processes.
    /// # Errors
    /// Returns the Postgres error when the lease transaction fails.
    #[allow(clippy::too_many_lines)]
    pub fn claim_generation_job(
        &mut self,
        worker_id: &str,
        now_ms: i64,
        lease_ms: i64,
        reclaim_grace_ms: i64,
        max_concurrent: i64,
        max_attempts: i32,
    ) -> Result<Option<PostgresGenerationJob>, PostgresStoreError> {
        let lease_token = format!(
            "{worker_id}:{}:{}",
            now_ms,
            NEXT_LEASE_TOKEN.fetch_add(1, Ordering::Relaxed)
        );
        let mut client = self.client.borrow_mut();
        let mut transaction = client.transaction()?;
        transaction.query_one(
            "SELECT pg_advisory_xact_lock($1::BIGINT)",
            &[&9_301_093_i64],
        )?;
        transaction.execute(
            "UPDATE memory_engine_generation_job_attempts attempt
             SET status = 'stale',
                 cost_usd_micros = CASE
                     WHEN attempt.generation_run_id IS NOT NULL AND EXISTS (
                         SELECT 1 FROM memory_engine_generation_runs run
                         WHERE run.account_id = attempt.account_id
                           AND run.generation_run_id = attempt.generation_run_id
                     ) THEN COALESCE((
                         SELECT (run->'usage'->>'costUsdMicros')::BIGINT
                         FROM memory_engine_generation_runs run
                         WHERE run.account_id = attempt.account_id
                           AND run.generation_run_id = attempt.generation_run_id
                     ), GREATEST(attempt.reservation_cost_usd_micros,
                                 attempt.reserved_cost_usd_micros))
                     ELSE GREATEST(attempt.cost_usd_micros, attempt.reserved_cost_usd_micros)
                 END,
                 reserved_cost_usd_micros = 0::BIGINT,
                 error = 'Lease expired before the provider attempt completed.',
                 completed_at_ms = $1::BIGINT, updated_at_ms = $1::BIGINT
             FROM memory_engine_generation_jobs job
             WHERE job.account_id = attempt.account_id AND job.job_id = attempt.job_id
               AND job.status = 'running' AND job.attempts = attempt.attempt
               AND job.lease_token = attempt.lease_token
               AND (job.lease_expires_at_ms IS NULL
                    OR job.lease_expires_at_ms + $2::BIGINT < $1::BIGINT)
               AND attempt.status = 'running'",
            &[&now_ms, &reclaim_grace_ms],
        )?;
        transaction.execute(
            "UPDATE memory_engine_generation_jobs
             SET status = 'failed', error = 'Maximum generation attempts exhausted.',
                 updated_at_ms = $1::BIGINT, lease_owner = NULL::TEXT,
                 lease_expires_at_ms = NULL::BIGINT, lease_token = NULL::TEXT,
                 reserved_cost_usd_micros = 0::BIGINT
             WHERE status IN ('running', 'retry') AND attempts >= $2::INTEGER
               AND (status = 'retry'
                    OR lease_expires_at_ms IS NULL
                    OR lease_expires_at_ms + $3::BIGINT < $1::BIGINT)",
            &[&now_ms, &max_attempts, &reclaim_grace_ms],
        )?;
        transaction.execute(
            "UPDATE memory_engine_generation_jobs
             SET status = 'retry', error = 'Lease expired; recovery is retrying this job.',
                 retry_at_ms = $1::BIGINT, updated_at_ms = $1::BIGINT,
                 lease_owner = NULL::TEXT, lease_expires_at_ms = NULL::BIGINT,
                 lease_token = NULL::TEXT
             WHERE status = 'running' AND attempts < $2::INTEGER
               AND (lease_expires_at_ms IS NULL
                    OR lease_expires_at_ms + $3::BIGINT < $1::BIGINT)",
            &[&now_ms, &max_attempts, &reclaim_grace_ms],
        )?;
        let row = transaction.query_opt(
            "WITH candidate AS (
                SELECT job.account_id, job.job_id
                FROM memory_engine_generation_jobs job
                WHERE (job.status = 'queued' OR (job.status = 'retry' AND job.retry_at_ms <= $1::BIGINT)
                       OR (job.status = 'running'
                           AND (job.lease_expires_at_ms IS NULL
                                OR job.lease_expires_at_ms + $3::BIGINT < $1::BIGINT)))
                  AND job.attempts < $2::INTEGER
                  AND (SELECT COUNT(*) FROM memory_engine_generation_jobs
                       WHERE status = 'running') < $4::BIGINT
                  AND (SELECT COUNT(*) FROM memory_engine_generation_jobs running
                       WHERE running.status = 'running'
                         AND running.account_id = job.account_id) < 1
                ORDER BY job.created_at_ms, job.account_id, job.job_id
                FOR UPDATE SKIP LOCKED LIMIT 1
             )
             UPDATE memory_engine_generation_jobs job
             SET status = 'running', attempts = job.attempts + 1, error = NULL::TEXT,
                 retry_at_ms = NULL::BIGINT, lease_owner = $5::TEXT,
                 lease_expires_at_ms = $1::BIGINT + $6::BIGINT, lease_token = $7::TEXT,
                 updated_at_ms = $1::BIGINT
             FROM candidate
             WHERE job.account_id = candidate.account_id AND job.job_id = candidate.job_id
             RETURNING job.account_id, job.job_id, job.source_id, job.title, job.status,
                       job.card_count, job.attempts, job.error, job.model_key,
                       job.cost_usd_micros, job.created_at_ms, job.updated_at_ms,
                       job.retry_at_ms, job.lease_expires_at_ms, job.lease_token,
                       job.reserved_cost_usd_micros",
            &[
                &now_ms,
                &max_attempts,
                &reclaim_grace_ms,
                &max_concurrent,
                &worker_id,
                &lease_ms,
                &lease_token,
            ],
        )?;
        if let Some(row) = row.as_ref() {
            transaction.execute(
                "INSERT INTO memory_engine_generation_job_attempts
                    (account_id, job_id, attempt, lease_token, status,
                     reservation_cost_usd_micros, reserved_cost_usd_micros,
                     cost_usd_micros, started_at_ms, updated_at_ms)
                 VALUES ($1::TEXT, $2::TEXT, $3::INTEGER, $4::TEXT, 'running',
                         $5::BIGINT, $5::BIGINT, 0::BIGINT, $6::BIGINT, $6::BIGINT)",
                &[
                    &row.get::<_, String>(0),
                    &row.get::<_, String>(1),
                    &row.get::<_, i32>(6),
                    &row.get::<_, String>(14),
                    &row.get::<_, i64>(15),
                    &now_ms,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(row.as_ref().map(generation_job_from_row))
    }

    /// # Errors
    /// Returns the Postgres error when the lease update fails.
    // The arguments mirror the fenced SQL boundary: account/job identity,
    // claim token, clock, outcome, and retry policy.
    #[allow(clippy::too_many_lines)]
    #[allow(clippy::too_many_arguments)]
    pub fn finish_generation_job(
        &mut self,
        account_id: &str,
        job_id: &str,
        lease_token: &str,
        now_ms: i64,
        result: Result<(usize, i64), String>,
        max_attempts: i32,
        retry_delay_ms: i64,
    ) -> Result<bool, PostgresStoreError> {
        let (succeeded, card_count, cost, error, retry_at) = match result {
            Ok((card_count, cost)) => (true, card_count, cost, None, None),
            Err(error) => (
                false,
                0,
                0,
                Some(error),
                Some(now_ms.saturating_add(retry_delay_ms)),
            ),
        };
        let mut client = self.client.borrow_mut();
        let mut transaction = client.transaction()?;
        let attempt_row = transaction.query_opt(
            "SELECT attempt, reservation_cost_usd_micros, generation_run_id
             FROM memory_engine_generation_job_attempts
             WHERE account_id = $1::TEXT AND job_id = $2::TEXT
               AND lease_token = $3::TEXT
             ORDER BY attempt DESC LIMIT 1 FOR UPDATE",
            &[&account_id, &job_id, &lease_token],
        )?;
        let Some(attempt_row) = attempt_row else {
            transaction.rollback()?;
            return Ok(false);
        };
        let attempt: i32 = attempt_row.get(0);
        let reservation: i64 = attempt_row.get(1);
        let generation_run_id: Option<String> = attempt_row.get(2);
        let run_cost = generation_run_id.as_deref().and_then(|run_id| {
            transaction
                .query_opt(
                    "SELECT COALESCE((run->'usage'->>'costUsdMicros')::BIGINT, 0)::BIGINT
                     FROM memory_engine_generation_runs
                     WHERE account_id = $1::TEXT AND generation_run_id = $2::TEXT",
                    &[&account_id, &run_id],
                )
                .ok()
                .flatten()
                .map(|row| row.get(0))
        });
        let current = transaction.query_opt(
            "SELECT status, lease_token, lease_expires_at_ms
             FROM memory_engine_generation_jobs
             WHERE account_id = $1::TEXT AND job_id = $2::TEXT
             FOR UPDATE",
            &[&account_id, &job_id],
        )?;
        let fenced = current.as_ref().is_some_and(|row| {
            let status: String = row.get(0);
            let current_token: Option<String> = row.get(1);
            let expires: Option<i64> = row.get(2);
            status == "running"
                && current_token.as_deref() == Some(lease_token)
                && expires.is_some_and(|expires| expires > now_ms)
        });
        let attempt_status = if fenced {
            if succeeded {
                "succeeded"
            } else {
                "failed"
            }
        } else {
            "stale"
        };
        let charged_cost = if cost > 0 {
            cost
        } else if succeeded {
            run_cost.unwrap_or(reservation)
        } else {
            run_cost.unwrap_or(reservation).max(cost)
        };
        transaction.execute(
            FINISH_GENERATION_JOB_ATTEMPT_SQL,
            &[
                &attempt_status,
                &charged_cost,
                &error,
                &now_ms,
                &account_id,
                &job_id,
                &attempt,
                &lease_token,
            ],
        )?;
        let changed = if fenced {
            transaction.execute(
                FINISH_GENERATION_JOB_SQL,
                &[
                    &succeeded,
                    &i32::try_from(card_count).unwrap_or(i32::MAX),
                    &charged_cost,
                    &error,
                    &retry_at,
                    &now_ms,
                    &account_id,
                    &job_id,
                    &max_attempts,
                    &lease_token,
                    &reservation,
                ],
            )?
        } else {
            0
        };
        transaction.commit()?;
        Ok(changed == 1)
    }

    /// # Errors
    /// Returns the Postgres error when the retry update fails.
    pub fn retry_generation_job(
        &mut self,
        account_id: &str,
        job_id: &str,
        now_ms: i64,
        max_attempts: i32,
    ) -> Result<bool, PostgresStoreError> {
        let changed = self.client.borrow_mut().execute(
            "UPDATE memory_engine_generation_jobs
             SET status = 'queued', error = NULL::TEXT, retry_at_ms = NULL::BIGINT,
                 reserved_cost_usd_micros = COALESCE((
                     SELECT attempt.reservation_cost_usd_micros
                     FROM memory_engine_generation_job_attempts attempt
                     WHERE attempt.account_id = memory_engine_generation_jobs.account_id
                       AND attempt.job_id = memory_engine_generation_jobs.job_id
                     ORDER BY attempt.attempt DESC LIMIT 1
                 ), 0::BIGINT), updated_at_ms = $1::BIGINT
             WHERE account_id = $2::TEXT AND job_id = $3::TEXT
               AND status = 'failed' AND attempts < $4::INTEGER",
            &[&now_ms, &account_id, &job_id, &max_attempts],
        )?;
        Ok(changed == 1)
    }
}

fn generation_job_from_row(row: &postgres::Row) -> PostgresGenerationJob {
    let card_count: i32 = row.get(5);
    let attempts: i32 = row.get(6);
    PostgresGenerationJob {
        account_id: row.get(0),
        id: row.get(1),
        source_id: row.get(2),
        title: row.get(3),
        status: row.get(4),
        card_count: usize::try_from(card_count).unwrap_or(0),
        attempts: u32::try_from(attempts).unwrap_or(0),
        error: row.get(7),
        model_key: row.get(8),
        cost_usd_micros: row.get(9),
        created_at: row.get(10),
        updated_at: row.get(11),
        retry_at: row.get(12),
        lease_expires_at: row.get(13),
        lease_token: row.get(14),
        reserved_cost_usd_micros: row.get(15),
    }
}

#[derive(Clone)]
pub struct AccountStudyStore<'a> {
    client: &'a RefCell<CountingClient>,
    scope: AccountScope,
}

impl AccountStudyStore<'_> {
    /// Serialize every per-account study mutation across Postgres connections.
    ///
    /// The concept selector takes this same key before reading review units,
    /// schedules, sources, drafts, and attempts. Any writer that can change
    /// that selector must use this helper so those reads observe a committed
    /// serial position, including across API replicas.
    fn with_account_transaction<R>(
        &mut self,
        operation: impl FnOnce(&mut CountingTransaction<'_>) -> Result<R, PostgresStoreError>,
    ) -> Result<R, PostgresStoreError> {
        let account_id = self.scope.account_id.clone();
        let mut client = self.client.borrow_mut();
        let mut transaction = client.transaction()?;
        transaction.execute(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
            &[&account_id],
        )?;
        let result = operation(&mut transaction)?;
        transaction.commit()?;
        Ok(result)
    }

    /// Read all scoped study state in the beta-store snapshot shape.
    ///
    /// This is the bridge surface the HTTP study session needs before the API can
    /// stop opening the file-backed beta store directly.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when Postgres reads or JSON decoding fail.
    pub fn snapshot(&self) -> Result<BetaStoreSnapshot, PostgresStoreError> {
        let rows = self.client.borrow_mut().query(
            "SELECT kind, value
             FROM (
                SELECT 'source_document'::text AS kind, created_at_ms AS sort_at,
                       source_document_id::text AS sort_id, document AS value
                FROM memory_engine_source_documents WHERE account_id = $1
                UNION ALL
                SELECT 'reference_span', created_at_ms, reference_span_id::text, span
                FROM memory_engine_reference_spans WHERE account_id = $1
                UNION ALL
                SELECT 'generated_prompt_draft', created_at_ms, draft_id::text, draft
                FROM memory_engine_generated_prompt_drafts WHERE account_id = $1
                UNION ALL
                SELECT 'review_unit', created_at_ms, review_unit_id::text, record
                FROM memory_engine_review_units WHERE account_id = $1
                UNION ALL
                SELECT 'schedule', updated_at_ms, review_unit_id::text,
                       jsonb_build_object('reviewUnitId', review_unit_id, 'state', state)
                FROM memory_engine_schedules WHERE account_id = $1
                UNION ALL
                SELECT 'attempt', occurred_at_ms, lpad(attempt_id::text, 19, '0'), attempt
                FROM memory_engine_attempts WHERE account_id = $1
                UNION ALL
                SELECT 'generation_run', started_at_ms, generation_run_id::text, run
                FROM memory_engine_generation_runs WHERE account_id = $1
                UNION ALL
                SELECT 'content_feedback', occurred_at_ms, feedback_id::text, feedback
                FROM memory_engine_content_feedback WHERE account_id = $1
                UNION ALL
                SELECT 'applied_review', applied_at_ms, receipt_key::text,
                       jsonb_build_object(
                           'key', receipt_key,
                           'attempt', attempt,
                           'expectedPriorScheduleState', expected_prior_schedule_state,
                           'scheduleState', schedule_state
                       )
                FROM memory_engine_applied_reviews WHERE account_id = $1
                UNION ALL
                SELECT 'concept_reference_note', updated_at_ms, concept_key::text, note
                FROM memory_engine_concept_reference_notes WHERE account_id = $1
             ) AS snapshot_rows
             ORDER BY kind, sort_at, sort_id",
            &[&self.scope.account_id],
        )?;

        let mut snapshot = BetaStoreSnapshot {
            version: 1,
            ..BetaStoreSnapshot::default()
        };
        for row in rows {
            let kind: &str = row.get(0);
            let value: serde_json::Value = row.get(1);
            match kind {
                "source_document" => {
                    snapshot
                        .source_documents
                        .push(serde_json::from_value(value)?);
                }
                "reference_span" => {
                    snapshot
                        .reference_spans
                        .push(serde_json::from_value(value)?);
                }
                "generated_prompt_draft" => {
                    snapshot
                        .generated_prompt_drafts
                        .push(serde_json::from_value(value)?);
                }
                "review_unit" => {
                    snapshot.review_units.push(serde_json::from_value(value)?);
                }
                "schedule" => {
                    snapshot.schedules.push(serde_json::from_value(value)?);
                }
                "attempt" => {
                    snapshot.attempts.push(serde_json::from_value(value)?);
                }
                "generation_run" => {
                    snapshot
                        .generation_runs
                        .push(serde_json::from_value(value)?);
                }
                "content_feedback" => {
                    snapshot
                        .content_feedback
                        .push(serde_json::from_value(value)?);
                }
                "applied_review" => {
                    snapshot
                        .applied_reviews
                        .push(serde_json::from_value(value)?);
                }
                "concept_reference_note" => {
                    snapshot
                        .concept_reference_notes
                        .push(serde_json::from_value(value)?);
                }
                _ => unreachable!("snapshot query returned an unknown row kind"),
            }
        }
        Ok(snapshot)
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

    /// Create a review unit for the scoped account, or leave an existing unit
    /// untouched. Review-unit records contain server-owned mutable queue,
    /// prompt, archive, and snooze state; a caller can hold a stale snapshot,
    /// so conflict replacement is deliberately not part of this API.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when serialization or persistence fails.
    pub fn save_review_unit(
        &mut self,
        review_unit: &BetaReviewUnitRecord,
    ) -> Result<(), PostgresStoreError> {
        let value = serde_json::to_value(review_unit)?;
        let account_id = self.scope.account_id.clone();
        self.with_account_transaction(|transaction| {
            transaction.execute(
                "INSERT INTO memory_engine_review_units
                    (account_id, review_unit_id, record, created_at_ms, archived_at_ms)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (account_id, review_unit_id) DO NOTHING",
                &[
                    &account_id,
                    &review_unit.review_unit_id.as_str(),
                    &value,
                    &review_unit.created_at,
                    &review_unit.archived_at,
                ],
            )?;
            Ok(())
        })
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
        let account_id = self.scope.account_id.clone();
        let draft_id = draft_id.to_owned();
        self.with_account_transaction(|transaction| {
            let draft: GeneratedPromptDraft = transaction
                .query_opt(
                    "SELECT draft FROM memory_engine_generated_prompt_drafts
                     WHERE account_id = $1 AND draft_id = $2
                     FOR UPDATE",
                    &[&account_id, &draft_id],
                )?
                .map(|row| {
                    let value: serde_json::Value = row.get(0);
                    serde_json::from_value(value)
                })
                .transpose()?
                .ok_or_else(|| PostgresStoreError::UnknownGeneratedPromptDraft(draft_id.clone()))?;
            if draft.validation.status != GeneratedPromptValidationStatus::Accepted {
                return Err(PostgresStoreError::RejectedGeneratedPromptDraft);
            }
            let generation_run_exists = draft
                .generation_run_id
                .as_ref()
                .map(|run_id| {
                    transaction
                        .query_opt(
                            "SELECT 1 FROM memory_engine_generation_runs
                             WHERE account_id = $1 AND generation_run_id = $2",
                            &[&account_id, run_id],
                        )
                        .map(|row| row.is_some())
                })
                .transpose()?
                .unwrap_or(false);
            if !generation_run_exists {
                return Err(PostgresStoreError::MissingGenerationRunForAcceptedDraft);
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
            let value = serde_json::to_value(&review_unit)?;
            let inserted = transaction.execute(
                "INSERT INTO memory_engine_review_units
                    (account_id, review_unit_id, record, created_at_ms, archived_at_ms)
                 VALUES ($1, $2, $3, $4, NULL)
                 ON CONFLICT (account_id, review_unit_id) DO NOTHING",
                &[
                    &account_id,
                    &review_unit.review_unit_id.as_str(),
                    &value,
                    &review_unit.created_at,
                ],
            )?;
            if inserted == 0 {
                return review_unit_from_transaction(
                    transaction,
                    &account_id,
                    &review_unit.review_unit_id,
                );
            }
            if let Some(schedule_state) = initial_schedule_state.as_ref() {
                let schedule_value = serde_json::to_value(schedule_state)?;
                transaction.execute(
                    "INSERT INTO memory_engine_schedules
                        (account_id, review_unit_id, state, updated_at_ms)
                     VALUES ($1, $2, $3, $4)
                     ON CONFLICT (account_id, review_unit_id) DO UPDATE
                     SET state = EXCLUDED.state,
                         updated_at_ms = EXCLUDED.updated_at_ms",
                    &[
                        &account_id,
                        &review_unit.review_unit_id.as_str(),
                        &schedule_value,
                        &draft.created_at,
                    ],
                )?;
            }
            Ok(review_unit)
        })
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
        assert_non_blank(prompt_text, "Review unit prompt")?;
        assert_non_blank(expected_answer, "Review unit expected answer")?;
        let account_id = self.scope.account_id.clone();
        let review_unit_id = review_unit_id.clone();
        let prompt_text = prompt_text.to_owned();
        let expected_answer = expected_answer.to_owned();
        self.with_account_transaction(|transaction| {
            let mut review_unit =
                review_unit_from_transaction(transaction, &account_id, &review_unit_id)?;
            reject_archived(&review_unit)?;
            replace_prompt_text(&mut review_unit.prompt, &prompt_text);
            replace_prompt_answer(&mut review_unit.prompt, &expected_answer)?;
            let prompt = serde_json::to_value(&review_unit.prompt)?;
            if let Some(draft_id) = &review_unit.generated_prompt_draft_id {
                let mut draft: GeneratedPromptDraft = transaction
                    .query_opt(
                        "SELECT draft FROM memory_engine_generated_prompt_drafts
                         WHERE account_id = $1 AND draft_id = $2
                         FOR UPDATE",
                        &[&account_id, draft_id],
                    )?
                    .map(|row| {
                        let value: serde_json::Value = row.get(0);
                        serde_json::from_value(value)
                    })
                    .transpose()?
                    .ok_or_else(|| {
                        PostgresStoreError::UnknownGeneratedPromptDraft(draft_id.clone())
                    })?;
                replace_prompt_text(&mut draft.prompt, &prompt_text);
                replace_prompt_answer(&mut draft.prompt, &expected_answer)?;
                if !draft
                    .critique_notes
                    .iter()
                    .any(|note| note == "Learner edited approved wording.")
                {
                    draft
                        .critique_notes
                        .push("Learner edited approved wording.".to_owned());
                }
                let draft_value = serde_json::to_value(draft)?;
                transaction.execute(
                    "UPDATE memory_engine_generated_prompt_drafts
                     SET draft = $3
                     WHERE account_id = $1 AND draft_id = $2",
                    &[&account_id, draft_id, &draft_value],
                )?;
            }
            transaction.execute(
                "UPDATE memory_engine_review_units
                 SET record = jsonb_set(record, '{prompt}', $3, true)
                 WHERE account_id = $1 AND review_unit_id = $2",
                &[&account_id, &review_unit_id.as_str(), &prompt],
            )?;
            Ok(review_unit)
        })
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
        let account_id = self.scope.account_id.clone();
        let review_unit_id = review_unit_id.clone();
        self.with_account_transaction(|transaction| {
            let mut review_unit =
                review_unit_from_transaction(transaction, &account_id, &review_unit_id)?;
            review_unit.archived_at = Some(archived_at);
            transaction.execute(
                "UPDATE memory_engine_review_units
                 SET record = jsonb_set(record, '{archivedAt}', to_jsonb($3::BIGINT), true),
                     archived_at_ms = $3
                 WHERE account_id = $1 AND review_unit_id = $2",
                &[&account_id, &review_unit_id.as_str(), &archived_at],
            )?;
            Ok(review_unit)
        })
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
        let account_id = self.scope.account_id.clone();
        let review_unit_id = review_unit_id.clone();
        self.with_account_transaction(|transaction| {
            let mut review_unit =
                review_unit_from_transaction(transaction, &account_id, &review_unit_id)?;
            reject_archived(&review_unit)?;
            review_unit.snoozed_until = Some(snoozed_until);
            transaction.execute(
                "UPDATE memory_engine_review_units
                 SET record = jsonb_set(record, '{snoozedUntil}', to_jsonb($3::BIGINT), true)
                 WHERE account_id = $1 AND review_unit_id = $2",
                &[&account_id, &review_unit_id.as_str(), &snoozed_until],
            )?;
            Ok(review_unit)
        })
    }

    /// Move every non-archived review unit under one persisted concept key
    /// forward in one account-scoped transaction.
    ///
    /// The JSON record is updated as a set, so membership is evaluated by the
    /// persisted queue concept key and all matching rows commit together.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when the transaction or record decoding
    /// fails.
    pub fn snooze_review_units_for_concept_until(
        &mut self,
        concept_key: &str,
        snoozed_until: i64,
    ) -> Result<Vec<BetaReviewUnitRecord>, PostgresStoreError> {
        let account_id = self.scope.account_id.clone();
        let concept_key = concept_key.to_owned();
        self.with_account_transaction(|transaction| {
            let rows = transaction.query(
                "UPDATE memory_engine_review_units
                 SET record = jsonb_set(
                     record,
                     '{snoozedUntil}',
                     to_jsonb($3::BIGINT),
                     true
                 )
                 WHERE account_id = $1
                   AND archived_at_ms IS NULL
                   AND record->'queue'->>'conceptKey' = $2
                 RETURNING record",
                &[&account_id, &concept_key, &snoozed_until],
            )?;
            rows.into_iter()
                .map(|row| {
                    let value: serde_json::Value = row.get(0);
                    Ok(serde_json::from_value(value)?)
                })
                .collect::<Result<Vec<BetaReviewUnitRecord>, PostgresStoreError>>()
        })
    }

    /// Resolve and snooze the requested current review unit's whole concept
    /// in one account-scoped transaction.
    ///
    /// The requested row, its persisted concept key, and the due candidate
    /// chosen from the same locked snapshot are validated before any member is
    /// updated. A stale request therefore commits no partial concept deferral.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when the requested unit is stale, has no
    /// usable concept key, or the transaction cannot be committed.
    pub fn snooze_current_review_unit_concept_until(
        &mut self,
        review_unit_id: &str,
        now: i64,
        snoozed_until: i64,
    ) -> Result<Vec<BetaReviewUnitRecord>, PostgresStoreError> {
        let mut client = self.client.borrow_mut();
        let mut transaction = client.transaction()?;
        transaction.execute(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
            &[&self.scope.account_id],
        )?;
        // All Postgres study writers use with_account_transaction with this
        // exact account key. The selector's source, draft, and attempt reads
        // therefore have the same serial order as their competing writes.
        // Lock the complete active candidate set before resolving current.
        // Every competing review mutation updates one of these rows, so a
        // replica cannot change archive/current state between this read and
        // the concept update. Ordering both lock queries by id keeps two
        // account operations from acquiring row locks in opposite orders.
        let active_rows = transaction.query(
            "SELECT review_unit_id, record FROM memory_engine_review_units
             WHERE account_id = $1 AND archived_at_ms IS NULL
             ORDER BY review_unit_id
             FOR UPDATE",
            &[&self.scope.account_id],
        )?;
        let active_records = active_rows
            .into_iter()
            .map(|row| {
                let review_unit_id: String = row.get(0);
                let value: serde_json::Value = row.get(1);
                Ok((review_unit_id, serde_json::from_value(value)?))
            })
            .collect::<Result<Vec<(String, BetaReviewUnitRecord)>, PostgresStoreError>>()?;
        let Some((_, requested)) = active_records.iter().find(|(id, _)| id == review_unit_id)
        else {
            return Err(PostgresStoreError::UnknownReviewUnit(ReviewUnitId::new(
                review_unit_id,
            )));
        };
        let schedule_rows = transaction.query(
            "SELECT schedules.review_unit_id, schedules.state
             FROM memory_engine_schedules AS schedules
             INNER JOIN memory_engine_review_units AS units
               ON units.account_id = schedules.account_id
              AND units.review_unit_id = schedules.review_unit_id
             WHERE schedules.account_id = $1 AND units.archived_at_ms IS NULL
             ORDER BY schedules.review_unit_id
             FOR UPDATE OF schedules, units",
            &[&self.scope.account_id],
        )?;
        let schedules = schedule_rows
            .into_iter()
            .map(|row| {
                let review_unit_id: String = row.get(0);
                let value: serde_json::Value = row.get(1);
                Ok((review_unit_id, serde_json::from_value(value)?))
            })
            .collect::<Result<BTreeMap<String, ScheduleState>, PostgresStoreError>>()?;

        if !current_review_unit_matches(
            &mut transaction,
            &self.scope.account_id,
            &active_records,
            &schedules,
            review_unit_id,
            now,
        )? {
            return Err(PostgresStoreError::UnknownReviewUnit(ReviewUnitId::new(
                review_unit_id,
            )));
        }

        let concept_key = requested
            .queue
            .concept_key
            .as_deref()
            .filter(|key| !key.trim().is_empty())
            .map(str::to_owned)
            .ok_or(PostgresStoreError::NoConceptKey)?;

        let rows = transaction.query(
            "UPDATE memory_engine_review_units
             SET record = jsonb_set(
                 record,
                 '{snoozedUntil}',
                 to_jsonb($3::BIGINT),
                 true
             )
             WHERE account_id = $1
               AND archived_at_ms IS NULL
               AND record->'queue'->>'conceptKey' = $2
             RETURNING record",
            &[&self.scope.account_id, &concept_key, &snoozed_until],
        )?;
        let snoozed = rows
            .into_iter()
            .map(|row| {
                let value: serde_json::Value = row.get(0);
                Ok(serde_json::from_value(value)?)
            })
            .collect::<Result<Vec<BetaReviewUnitRecord>, PostgresStoreError>>()?;
        transaction.commit()?;

        Ok(snoozed)
    }

    /// Replace volatile lifecycle metadata without changing schedule history.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when the review unit is unknown.
    pub fn set_review_unit_lifecycle(
        &mut self,
        review_unit_id: &ReviewUnitId,
        lifecycle: ReviewUnitLifecycle,
    ) -> Result<BetaReviewUnitRecord, PostgresStoreError> {
        let account_id = self.scope.account_id.clone();
        let review_unit_id = review_unit_id.clone();
        self.with_account_transaction(|transaction| {
            let mut review_unit =
                review_unit_from_transaction(transaction, &account_id, &review_unit_id)?;
            reject_archived(&review_unit)?;
            review_unit.queue.lifecycle = lifecycle;
            let queue = serde_json::to_value(&review_unit.queue)?;
            transaction.execute(
                "UPDATE memory_engine_review_units
                 SET record = jsonb_set(record, '{queue}', $3, true)
                 WHERE account_id = $1 AND review_unit_id = $2",
                &[&account_id, &review_unit_id.as_str(), &queue],
            )?;
            Ok(review_unit)
        })
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
        let account_id = self.scope.account_id.clone();
        self.with_account_transaction(|transaction| {
            transaction.execute(
                "INSERT INTO memory_engine_source_documents
                    (account_id, source_document_id, document, created_at_ms)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT (account_id, source_document_id) DO UPDATE
                 SET document = EXCLUDED.document,
                     created_at_ms = EXCLUDED.created_at_ms",
                &[&account_id, &document.id, &value, &document.created_at],
            )?;
            Ok(())
        })
    }

    /// Save or replace a generated concept-level reference note for the scoped account.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when serialization or persistence fails.
    pub fn save_concept_reference_note(
        &mut self,
        note: &ConceptReferenceNote,
    ) -> Result<(), PostgresStoreError> {
        let value = serde_json::to_value(note)?;
        let account_id = self.scope.account_id.clone();
        self.with_account_transaction(|transaction| {
            transaction.execute(
                "INSERT INTO memory_engine_concept_reference_notes
                    (account_id, concept_key, note, updated_at_ms)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT (account_id, concept_key) DO UPDATE
                 SET note = EXCLUDED.note,
                     updated_at_ms = EXCLUDED.updated_at_ms",
                &[&account_id, &note.concept_key, &value, &note.updated_at],
            )?;
            Ok(())
        })
    }

    /// Hide source material from learner-facing flows while preserving receipts.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when the source is unknown or persistence fails.
    pub fn archive_source_document(
        &mut self,
        source_document_id: &str,
        archived_at: i64,
    ) -> Result<SourceDocument, PostgresStoreError> {
        let account_id = self.scope.account_id.clone();
        let source_document_id = source_document_id.to_owned();
        self.with_account_transaction(|transaction| {
            let mut document =
                source_document_from_transaction(transaction, &account_id, &source_document_id)?;
            document.archived_at = Some(archived_at);
            let value = serde_json::to_value(&document)?;
            transaction.execute(
                "UPDATE memory_engine_source_documents
                 SET document = $3
                 WHERE account_id = $1 AND source_document_id = $2",
                &[&account_id, &source_document_id, &value],
            )?;
            Ok(document)
        })
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
        let account_id = self.scope.account_id.clone();
        self.with_account_transaction(|transaction| {
            transaction.execute(
                "INSERT INTO memory_engine_reference_spans
                    (account_id, reference_span_id, source_document_id, span, created_at_ms)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (account_id, reference_span_id) DO UPDATE
                 SET source_document_id = EXCLUDED.source_document_id,
                     span = EXCLUDED.span,
                     created_at_ms = EXCLUDED.created_at_ms",
                &[
                    &account_id,
                    &reference.id,
                    &reference.source_document_id,
                    &value,
                    &reference.created_at,
                ],
            )?;
            Ok(())
        })
    }

    /// Save or replace a generation run receipt for the scoped account.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when serialization or persistence fails.
    pub fn save_generation_run(&mut self, run: &GenerationRun) -> Result<(), PostgresStoreError> {
        let value = serde_json::to_value(run)?;
        let account_id = self.scope.account_id.clone();
        self.with_account_transaction(|transaction| {
            transaction.execute(
                "INSERT INTO memory_engine_generation_runs
                    (account_id, generation_run_id, run, started_at_ms)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT (account_id, generation_run_id) DO UPDATE
                 SET run = EXCLUDED.run,
                     started_at_ms = EXCLUDED.started_at_ms",
                &[&account_id, &run.id, &value, &run.started_at],
            )?;
            Ok(())
        })
    }

    /// Check whether a generation run still belongs to the currently active
    /// job attempt for the scoped account and exact lease token.
    ///
    /// # Errors
    /// Returns [`PostgresStoreError`] when the receipt cannot be read.
    pub fn generation_job_attempt_has_commit_fence(
        &mut self,
        run_id: &str,
        attempt: i32,
        lease_token: &str,
        now_ms: i64,
    ) -> Result<bool, PostgresStoreError> {
        let row = self.client.borrow_mut().query_opt(
            "SELECT 1
             FROM memory_engine_generation_job_attempts attempt
             JOIN memory_engine_generation_jobs job
               ON job.account_id = attempt.account_id AND job.job_id = attempt.job_id
             WHERE attempt.account_id = $1::TEXT
               AND attempt.generation_run_id = $2::TEXT
               AND attempt.attempt = $3::INTEGER
               AND attempt.lease_token = $4::TEXT
               AND attempt.status = 'running'
               AND job.status = 'running'
               AND job.attempts = attempt.attempt
               AND job.lease_token = attempt.lease_token
               AND job.lease_expires_at_ms > $5::BIGINT
             FOR UPDATE OF attempt, job
             LIMIT 1",
            &[
                &self.scope.account_id,
                &run_id,
                &attempt,
                &lease_token,
                &now_ms,
            ],
        )?;
        Ok(row.is_some())
    }

    /// Execute a block of mutations inside one SQL transaction.
    ///
    /// # Errors
    /// Returns [`PostgresStoreError`] when the transaction cannot start, commit,
    /// or roll back.
    pub fn with_transaction<R>(
        &mut self,
        operation: impl FnOnce(&mut Self) -> Result<R, PostgresStoreError>,
    ) -> Result<R, PostgresStoreError> {
        self.client.borrow_mut().batch_execute("BEGIN")?;
        let result = operation(self);
        match result {
            Ok(value) => {
                self.client.borrow_mut().batch_execute("COMMIT")?;
                Ok(value)
            }
            Err(error) => {
                let _ = self.client.borrow_mut().batch_execute("ROLLBACK");
                Err(error)
            }
        }
    }

    /// Start a SQL transaction on the scoped account connection.
    ///
    /// # Errors
    /// Returns [`PostgresStoreError`] when Postgres rejects the command.
    pub fn begin_transaction(&mut self) -> Result<(), PostgresStoreError> {
        self.client.borrow_mut().batch_execute("BEGIN")?;
        Ok(())
    }

    /// Commit the current SQL transaction on the scoped account connection.
    ///
    /// # Errors
    /// Returns [`PostgresStoreError`] when Postgres rejects the command.
    pub fn commit_transaction(&mut self) -> Result<(), PostgresStoreError> {
        self.client.borrow_mut().batch_execute("COMMIT")?;
        Ok(())
    }

    /// Roll back the current SQL transaction on the scoped account connection.
    ///
    /// # Errors
    /// Returns [`PostgresStoreError`] when Postgres rejects the command.
    pub fn rollback_transaction(&mut self) -> Result<(), PostgresStoreError> {
        self.client.borrow_mut().batch_execute("ROLLBACK")?;
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
        let account_id = self.scope.account_id.clone();
        self.with_account_transaction(|transaction| {
            transaction.execute(
                "INSERT INTO memory_engine_generated_prompt_drafts
                    (account_id, draft_id, review_unit_id, draft, created_at_ms)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (account_id, draft_id) DO UPDATE
                 SET review_unit_id = EXCLUDED.review_unit_id,
                     draft = EXCLUDED.draft,
                     created_at_ms = EXCLUDED.created_at_ms",
                &[
                    &account_id,
                    &draft.id,
                    &draft.review_unit_id.as_str(),
                    &value,
                    &draft.created_at,
                ],
            )?;
            Ok(())
        })
    }

    /// Append one account-scoped learner content judgment. Replaying the same
    /// feedback id is idempotent; a different payload under that id is rejected.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when the account or review unit is
    /// invalid, the idempotency payload conflicts, or the transaction fails.
    pub fn record_content_feedback(
        &mut self,
        feedback: &ContentFeedback,
    ) -> Result<ContentFeedback, PostgresStoreError> {
        if feedback.account_id != self.scope.account_id {
            return Err(PostgresStoreError::FeedbackAccountMismatch);
        }

        let value = serde_json::to_value(feedback)?;
        let mut client = self.client.borrow_mut();
        let mut transaction = client.transaction()?;
        let known = transaction.query_opt(
            "SELECT 1 FROM memory_engine_review_units
             WHERE account_id = $1 AND review_unit_id = $2
             FOR UPDATE",
            &[&self.scope.account_id, &feedback.review_unit_id.as_str()],
        )?;
        if known.is_none() {
            return Err(PostgresStoreError::UnknownReviewUnit(
                feedback.review_unit_id.clone(),
            ));
        }
        if let Some(row) = transaction.query_opt(
            "SELECT feedback FROM memory_engine_content_feedback
             WHERE account_id = $1 AND feedback_id = $2",
            &[&self.scope.account_id, &feedback.id],
        )? {
            let existing: ContentFeedback = serde_json::from_value(row.get(0))?;
            if content_feedback_replay_matches(&existing, feedback) {
                transaction.rollback()?;
                return Ok(existing);
            }
            transaction.rollback()?;
            return Err(PostgresStoreError::DuplicateContentFeedback(
                feedback.id.clone(),
            ));
        }
        if let Some(supersedes_id) = &feedback.supersedes_id {
            let row = transaction.query_opt(
                "SELECT review_unit_id FROM memory_engine_content_feedback
                 WHERE account_id = $1 AND feedback_id = $2",
                &[&self.scope.account_id, supersedes_id],
            )?;
            let Some(row) = row else {
                return Err(PostgresStoreError::FeedbackSupersedesUnknown(
                    supersedes_id.clone(),
                ));
            };
            let superseded_review_unit_id: String = row.get(0);
            if superseded_review_unit_id != feedback.review_unit_id.as_str() {
                return Err(PostgresStoreError::FeedbackSupersedesOtherReviewUnit(
                    supersedes_id.clone(),
                ));
            }
        }
        let current_head: Option<String> = transaction
            .query_opt(
                "SELECT candidate.feedback_id
                 FROM memory_engine_content_feedback AS candidate
                 WHERE candidate.account_id = $1
                   AND candidate.review_unit_id = $2
                   AND NOT EXISTS (
                       SELECT 1
                       FROM memory_engine_content_feedback AS child
                       WHERE child.account_id = candidate.account_id
                         AND child.review_unit_id = candidate.review_unit_id
                         AND child.feedback::jsonb->>'supersedesId' = candidate.feedback_id
                   )
                 ORDER BY candidate.occurred_at_ms DESC, candidate.feedback_id DESC
                 LIMIT 1",
                &[&self.scope.account_id, &feedback.review_unit_id.as_str()],
            )?
            .map(|row| row.get(0));
        if feedback.supersedes_id.as_deref() != current_head.as_deref() {
            return Err(PostgresStoreError::FeedbackSupersedesStale {
                expected_head: current_head,
                supplied_parent: feedback.supersedes_id.clone(),
            });
        }

        transaction.execute(
            "INSERT INTO memory_engine_content_feedback
                (account_id, feedback_id, review_unit_id, feedback, occurred_at_ms)
             VALUES ($1, $2, $3, $4, $5)",
            &[
                &self.scope.account_id,
                &feedback.id,
                &feedback.review_unit_id.as_str(),
                &value,
                &feedback.occurred_at,
            ],
        )?;
        transaction.commit()?;

        Ok(feedback.clone())
    }

    /// Export active feedback using the same resolved provenance contract as
    /// the file-backed store.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresStoreError`] when the scoped snapshot cannot be read
    /// or feedback provenance cannot be resolved.
    pub fn export_content_feedback_json(&self) -> Result<String, PostgresStoreError> {
        let snapshot = self.snapshot()?;
        memory_engine_persistence::export_content_feedback_json(&snapshot)
            .map_err(|error| PostgresStoreError::StudySession(error.to_string()))
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
        let account_id = self.scope.account_id.clone();
        let review_unit_id = review_unit_id.clone();
        let schedule_value = schedule_state.map(serde_json::to_value).transpose()?;
        self.with_account_transaction(|transaction| {
            assert_known_review_unit_in_transaction(transaction, &account_id, &review_unit_id)?;
            if let Some(value) = schedule_value {
                transaction.execute(
                    "INSERT INTO memory_engine_schedules
                        (account_id, review_unit_id, state, updated_at_ms)
                     VALUES ($1, $2, $3, $4)
                     ON CONFLICT (account_id, review_unit_id) DO UPDATE
                     SET state = EXCLUDED.state,
                         updated_at_ms = EXCLUDED.updated_at_ms",
                    &[
                        &account_id,
                        &review_unit_id.as_str(),
                        &value,
                        &updated_at_ms,
                    ],
                )?;
            } else {
                transaction.execute(
                    "DELETE FROM memory_engine_schedules
                     WHERE account_id = $1 AND review_unit_id = $2",
                    &[&account_id, &review_unit_id.as_str()],
                )?;
            }
            Ok(())
        })
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

    fn save_concept_reference_note(
        &mut self,
        note: ConceptReferenceNote,
    ) -> Result<ConceptReferenceNote, Self::Error> {
        AccountStudyStore::save_concept_reference_note(self, &note)?;

        Ok(note)
    }

    fn save_generated_prompt_draft(
        &mut self,
        draft: GeneratedPromptDraft,
    ) -> Result<GeneratedPromptDraft, Self::Error> {
        AccountStudyStore::save_generated_prompt_draft(self, &draft)?;

        Ok(draft)
    }
}

impl ContentFeedbackStore for AccountStudyStore<'_> {
    type Error = PostgresStoreError;

    fn record_content_feedback(
        &mut self,
        feedback: ContentFeedback,
    ) -> Result<ContentFeedback, Self::Error> {
        AccountStudyStore::record_content_feedback(self, &feedback)
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

    fn archive_source_document(
        &mut self,
        source_document_id: &str,
        archived_at: i64,
    ) -> Result<SourceDocument, <Self as MemoryServiceStore>::Error> {
        AccountStudyStore::archive_source_document(self, source_document_id, archived_at)
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

    fn snooze_review_units_for_concept_until(
        &mut self,
        concept_key: &str,
        snoozed_until: i64,
    ) -> Result<Vec<BetaReviewUnitRecord>, <Self as MemoryServiceStore>::Error> {
        AccountStudyStore::snooze_review_units_for_concept_until(self, concept_key, snoozed_until)
    }

    fn set_review_unit_lifecycle(
        &mut self,
        review_unit_id: &ReviewUnitId,
        lifecycle: ReviewUnitLifecycle,
    ) -> Result<BetaReviewUnitRecord, <Self as MemoryServiceStore>::Error> {
        AccountStudyStore::set_review_unit_lifecycle(self, review_unit_id, lifecycle)
    }
}

fn assert_known_review_unit_in_transaction(
    transaction: &mut CountingTransaction<'_>,
    account_id: &str,
    review_unit_id: &ReviewUnitId,
) -> Result<(), PostgresStoreError> {
    let known = transaction.query_opt(
        "SELECT 1 FROM memory_engine_review_units
         WHERE account_id = $1 AND review_unit_id = $2",
        &[&account_id, &review_unit_id.as_str()],
    )?;
    known
        .is_some()
        .then_some(())
        .ok_or_else(|| PostgresStoreError::UnknownReviewUnit(review_unit_id.clone()))
}

fn review_unit_from_transaction(
    transaction: &mut CountingTransaction<'_>,
    account_id: &str,
    review_unit_id: &ReviewUnitId,
) -> Result<BetaReviewUnitRecord, PostgresStoreError> {
    let row = transaction.query_opt(
        "SELECT record FROM memory_engine_review_units
         WHERE account_id = $1 AND review_unit_id = $2
         FOR UPDATE",
        &[&account_id, &review_unit_id.as_str()],
    )?;
    let Some(row) = row else {
        return Err(PostgresStoreError::UnknownReviewUnit(
            review_unit_id.clone(),
        ));
    };
    let value: serde_json::Value = row.get(0);
    Ok(serde_json::from_value(value)?)
}

fn source_document_from_transaction(
    transaction: &mut CountingTransaction<'_>,
    account_id: &str,
    source_document_id: &str,
) -> Result<SourceDocument, PostgresStoreError> {
    let row = transaction.query_opt(
        "SELECT document FROM memory_engine_source_documents
         WHERE account_id = $1 AND source_document_id = $2
         FOR UPDATE",
        &[&account_id, &source_document_id],
    )?;
    let Some(row) = row else {
        return Err(PostgresStoreError::UnknownSourceDocument(
            source_document_id.to_owned(),
        ));
    };
    let value: serde_json::Value = row.get(0);
    Ok(serde_json::from_value(value)?)
}

fn current_review_unit_matches(
    transaction: &mut CountingTransaction<'_>,
    account_id: &str,
    active_records: &[(String, BetaReviewUnitRecord)],
    schedules: &BTreeMap<String, ScheduleState>,
    requested_id: &str,
    now: i64,
) -> Result<bool, PostgresStoreError> {
    let candidates = active_records
        .iter()
        .map(|(review_unit_id, record)| {
            let schedule = schedules.get(review_unit_id).cloned();
            let mut candidate = record.queue.with_schedule(schedule);
            if let Some(deferred_until) = record.snoozed_until {
                candidate = defer_queue_availability(&candidate, deferred_until);
            }
            candidate
        })
        .collect::<Vec<_>>();
    let source_documents = transaction
        .query(
            "SELECT document FROM memory_engine_source_documents
             WHERE account_id = $1
             ORDER BY created_at_ms, source_document_id",
            &[&account_id],
        )?
        .into_iter()
        .map(|row| {
            let value: serde_json::Value = row.get(0);
            Ok(serde_json::from_value(value)?)
        })
        .collect::<Result<Vec<SourceDocument>, PostgresStoreError>>()?;
    let generated_prompt_drafts = transaction
        .query(
            "SELECT draft FROM memory_engine_generated_prompt_drafts
             WHERE account_id = $1
             ORDER BY created_at_ms, draft_id",
            &[&account_id],
        )?
        .into_iter()
        .map(|row| {
            let value: serde_json::Value = row.get(0);
            Ok(serde_json::from_value(value)?)
        })
        .collect::<Result<Vec<GeneratedPromptDraft>, PostgresStoreError>>()?;
    let attempts = transaction
        .query(
            "SELECT attempt FROM memory_engine_attempts
             WHERE account_id = $1
             ORDER BY occurred_at_ms, attempt_id",
            &[&account_id],
        )?
        .into_iter()
        .map(|row| {
            let value: serde_json::Value = row.get(0);
            Ok(serde_json::from_value(value)?)
        })
        .collect::<Result<Vec<ServiceAttemptRecord>, PostgresStoreError>>()?;
    let snapshot = BetaStoreSnapshot {
        version: 1,
        source_documents,
        reference_spans: Vec::new(),
        generated_prompt_drafts,
        review_units: active_records
            .iter()
            .map(|(_, record)| record.clone())
            .collect(),
        schedules: schedules
            .iter()
            .map(|(review_unit_id, state)| ScheduleRecord {
                review_unit_id: ReviewUnitId::new(review_unit_id.clone()),
                state: state.clone(),
            })
            .collect(),
        attempts,
        generation_runs: Vec::new(),
        content_feedback: Vec::new(),
        applied_reviews: Vec::new(),
        concept_reference_notes: Vec::new(),
    };

    Ok(select_current_review_unit(&snapshot, &candidates, now)
        .as_ref()
        .map(ReviewUnitId::as_str)
        == Some(requested_id))
}

impl MemoryServiceStore for AccountStudyStore<'_> {
    type Error = PostgresStoreError;

    fn record_attempt(&mut self, attempt: ServiceAttemptRecord) -> Result<(), Self::Error> {
        let value = serde_json::to_value(&attempt)?;
        let account_id = self.scope.account_id.clone();
        self.with_account_transaction(|transaction| {
            assert_known_review_unit_in_transaction(
                transaction,
                &account_id,
                &attempt.review_unit_id,
            )?;
            transaction.execute(
                "INSERT INTO memory_engine_attempts
                    (account_id, review_unit_id, prompt_id, idempotency_key, attempt, occurred_at_ms)
                 VALUES ($1, $2, $3, $4, $5, $6)",
                &[
                    &account_id,
                    &attempt.review_unit_id.as_str(),
                    &attempt.prompt_id,
                    &attempt.idempotency_key,
                    &value,
                    &attempt.occurred_at,
                ],
            )?;
            Ok(())
        })
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
        transaction.execute(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
            &[&self.scope.account_id],
        )?;
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
            "SELECT review_units.record, schedules.state
             FROM memory_engine_review_units AS review_units
             LEFT JOIN memory_engine_schedules AS schedules
               ON schedules.account_id = review_units.account_id
              AND schedules.review_unit_id = review_units.review_unit_id
             WHERE review_units.account_id = $1
               AND review_units.archived_at_ms IS NULL
             ORDER BY review_units.created_at_ms, review_units.review_unit_id",
            &[&self.scope.account_id],
        )?;

        rows.into_iter()
            .map(|row| {
                let record: BetaReviewUnitRecord =
                    serde_json::from_value(row.get::<_, serde_json::Value>(0))?;
                let schedule_state = row
                    .get::<_, Option<serde_json::Value>>(1)
                    .map(serde_json::from_value)
                    .transpose()?;
                let mut candidate = record.queue.with_schedule(schedule_state);
                if let Some(snoozed_until) = record.snoozed_until {
                    candidate = defer_queue_availability(&candidate, snoozed_until);
                }
                Ok(candidate)
            })
            .collect()
    }
}

#[derive(Debug)]
pub enum PostgresStoreError {
    BlankAccountId,
    Blank {
        label: &'static str,
    },
    InvalidBooleanAnswer,
    NoConceptKey,
    UnknownSourceDocument(String),
    UnknownReviewUnit(ReviewUnitId),
    ReviewUnitArchived(ReviewUnitId),
    UnknownGeneratedPromptDraft(String),
    RejectedGeneratedPromptDraft,
    MissingGenerationRunForAcceptedDraft,
    ReviewUnitMismatch,
    ScheduleLastReviewMismatch,
    DuplicateAppliedReview(String),
    StaleScheduleWrite(ReviewUnitId),
    FeedbackAccountMismatch,
    DuplicateContentFeedback(String),
    FeedbackSupersedesUnknown(String),
    FeedbackSupersedesOtherReviewUnit(String),
    FeedbackSupersedesOtherAccount(String),
    FeedbackSupersedesStale {
        expected_head: Option<String>,
        supplied_parent: Option<String>,
    },
    StudySession(String),
    Postgres(postgres::Error),
    Json(serde_json::Error),
}

impl fmt::Display for PostgresStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BlankAccountId => formatter.write_str("Account id must not be blank"),
            Self::Blank { label } => write!(formatter, "{label} must not be blank"),
            Self::InvalidBooleanAnswer => {
                formatter.write_str("Boolean answers must be true or false")
            }
            Self::NoConceptKey => formatter.write_str("The active review unit has no concept key"),
            Self::UnknownSourceDocument(id) => write!(formatter, "Unknown source document: {id}"),
            Self::UnknownReviewUnit(id) => write!(formatter, "Unknown review unit: {id}"),
            Self::ReviewUnitArchived(id) => write!(formatter, "Review unit is archived: {id}"),
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
            Self::FeedbackAccountMismatch => {
                formatter.write_str("Content feedback account does not match the store scope")
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
            | Self::Blank { .. }
            | Self::InvalidBooleanAnswer
            | Self::NoConceptKey
            | Self::UnknownSourceDocument(_)
            | Self::UnknownReviewUnit(_)
            | Self::ReviewUnitArchived(_)
            | Self::UnknownGeneratedPromptDraft(_)
            | Self::RejectedGeneratedPromptDraft
            | Self::MissingGenerationRunForAcceptedDraft
            | Self::ReviewUnitMismatch
            | Self::ScheduleLastReviewMismatch
            | Self::DuplicateAppliedReview(_)
            | Self::StaleScheduleWrite(_)
            | Self::FeedbackAccountMismatch
            | Self::DuplicateContentFeedback(_)
            | Self::FeedbackSupersedesUnknown(_)
            | Self::FeedbackSupersedesOtherReviewUnit(_)
            | Self::FeedbackSupersedesOtherAccount(_)
            | Self::FeedbackSupersedesStale { .. }
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
            memory_engine_study::BetaStudyError::NoConceptKey => Self::NoConceptKey,
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

fn assert_non_blank(value: &str, label: &'static str) -> Result<(), PostgresStoreError> {
    if value.trim().is_empty() {
        return Err(PostgresStoreError::Blank { label });
    }
    Ok(())
}

fn reject_archived(review_unit: &BetaReviewUnitRecord) -> Result<(), PostgresStoreError> {
    review_unit
        .archived_at
        .is_none()
        .then_some(())
        .ok_or_else(|| PostgresStoreError::ReviewUnitArchived(review_unit.review_unit_id.clone()))
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

fn replace_prompt_answer(prompt: &mut Prompt, answer: &str) -> Result<(), PostgresStoreError> {
    match prompt {
        Prompt::Mcq {
            choices,
            correct_choice,
            ..
        } => {
            answer.clone_into(correct_choice);
            if !choices.iter().any(|choice| choice == answer) {
                choices.push(answer.to_owned());
            }
        }
        Prompt::Boolean { correct_answer, .. } => {
            *correct_answer = parse_strict_boolean_answer(answer)
                .ok_or(PostgresStoreError::InvalidBooleanAnswer)?;
        }
        Prompt::Exact(prompt) => {
            prompt.accepted_answers = vec![answer.to_owned()];
        }
    }
    Ok(())
}

#[must_use]
pub fn migration_sql() -> &'static str {
    MIGRATION_SQL.as_str()
}

#[must_use]
pub fn generation_jobs_migration_sql() -> &'static str {
    GENERATION_JOBS_MIGRATION_SQL_COMPLETE.as_str()
}

#[must_use]
pub fn applied_review_receipt_key(attempt: &ServiceAttemptRecord) -> String {
    applied_review_key(attempt)
}

#[allow(dead_code)]
fn _receipt_type_anchor(_: &AppliedReviewReceipt) {}

#[cfg(test)]
mod tests {
    use super::{replace_prompt_answer, replace_prompt_text};
    use memory_engine_core::{
        ExactPrompt, ExactPromptKind, Prompt, ReviewUnitId, ReviewUnitLifecycle, ScheduleState,
        ScheduleStatus,
    };

    #[test]
    fn retry_connect_recovers_from_transient_failures() {
        let mut attempts = 0;
        let result = super::retry_connect(|| {
            attempts += 1;
            if attempts < 3 {
                Err("transient")
            } else {
                Ok("connected")
            }
        });
        assert_eq!(result, Ok("connected"));
        assert_eq!(attempts, 3);
    }

    #[test]
    fn retry_connect_gives_up_after_the_attempt_budget() {
        let mut attempts = 0;
        let result: Result<(), &str> = super::retry_connect(|| {
            attempts += 1;
            Err("hard down")
        });
        assert_eq!(result, Err("hard down"));
        assert_eq!(attempts, super::CONNECT_ATTEMPTS);
    }
    #[test]
    fn counting_client_counts_direct_and_transaction_calls_exactly() {
        let Some(database_url) = std::env::var("MEMORY_ENGINE_POSTGRES_TEST_URL").ok() else {
            eprintln!("skipping counting client test; MEMORY_ENGINE_POSTGRES_TEST_URL is unset");
            return;
        };
        let client = crate::connect_client(&database_url).expect("connect counting test client");
        let mut client = super::CountingClient::new(client);

        assert_eq!(client.statement_count(), 0);
        client
            .query("SELECT 1", &[])
            .expect("direct query succeeds");
        client
            .query_one("SELECT 1", &[])
            .expect("direct query_one succeeds");
        client
            .query_opt("SELECT 1", &[])
            .expect("direct query_opt succeeds");
        client
            .execute("SELECT 1", &[])
            .expect("direct execute succeeds");
        client
            .batch_execute("SELECT 1")
            .expect("direct batch_execute succeeds");
        assert_eq!(client.statement_count(), 5);

        let statement_count = Arc::clone(&client.statement_count);
        let mut transaction = client.transaction().expect("transaction begins");
        assert_eq!(statement_count.load(Ordering::Relaxed), 6);
        transaction
            .execute("SELECT 1", &[])
            .expect("transaction body succeeds");
        assert_eq!(statement_count.load(Ordering::Relaxed), 7);
        transaction.commit().expect("transaction commits");
        assert_eq!(statement_count.load(Ordering::Relaxed), 8);

        let transaction = client.transaction().expect("rollback transaction begins");
        assert_eq!(statement_count.load(Ordering::Relaxed), 9);
        transaction
            .rollback()
            .expect("explicit transaction rollback succeeds");
        assert_eq!(statement_count.load(Ordering::Relaxed), 10);

        let mut transaction = client
            .transaction()
            .expect("implicit rollback transaction begins");
        assert_eq!(statement_count.load(Ordering::Relaxed), 11);
        transaction
            .execute("SELECT 1", &[])
            .expect("implicit rollback body succeeds");
        assert_eq!(statement_count.load(Ordering::Relaxed), 12);
        drop(transaction);
        assert_eq!(statement_count.load(Ordering::Relaxed), 13);
    }

    #[test]
    fn statement_count_saturates_at_u64_max() {
        let counter = std::sync::atomic::AtomicU64::new(u64::MAX - 1);
        super::increment_statement_count(&counter);
        super::increment_statement_count(&counter);
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
    }

    #[test]
    fn study_store_exposes_cumulative_statement_count() {
        let Some(database_url) = std::env::var("MEMORY_ENGINE_POSTGRES_TEST_URL").ok() else {
            eprintln!(
                "skipping study store statement count test; MEMORY_ENGINE_POSTGRES_TEST_URL is unset"
            );
            return;
        };
        let mut store = super::PostgresStudyStore::connect(&database_url)
            .expect("connect statement count store");
        assert_eq!(store.statement_count(), 0);
        store.ping().expect("ping succeeds");
        assert_eq!(store.statement_count(), 1);
        store.ping().expect("second ping succeeds");
        assert_eq!(store.statement_count(), 2);
    }

    use memory_engine_persistence::{
        BetaReviewUnitRecord, BetaStoreSnapshot, GeneratedLearningActivityKind,
        GeneratedPromptDraft, GeneratedPromptModel, GeneratedPromptValidation,
        GeneratedPromptValidationStatus, GenerationRun, GenerationRunUsage,
        PersistedQueueCandidate, ReferenceSpan, SourceDocument, SourceDocumentKind,
        SourcePermission,
    };
    use memory_engine_service::{
        record_content_feedback, ContentFeedbackError, ContentFeedbackVerdict,
        RecordContentFeedbackCommand, ServiceAttemptRecord,
    };
    use memory_engine_study::{BetaStudySession, BetaStudySourceInput, BetaStudyStatus};
    use std::sync::{atomic::Ordering, Arc, Barrier};

    use super::{
        applied_review_receipt_key, generation_jobs_migration_sql, migration_sql, AccountScope,
        AccountStudyStore, MemoryServiceStore, PostgresEnqueueOutcome, PostgresStoreError,
        PostgresStudyStore, CLAIM_RETURN_NOTIFICATION_SQL, FINISH_GENERATION_JOB_SQL,
        RENEW_GENERATION_JOB_SQL,
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
        let jobs_sql = generation_jobs_migration_sql();

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
        assert!(sql.contains("memory_engine_content_feedback"));
        assert!(sql.contains("PRIMARY KEY (account_id, feedback_id)"));
        assert!(sql.contains("challenge_hash TEXT PRIMARY KEY"));
        assert!(sql.contains("consumed_at_ms BIGINT"));
        assert!(sql.contains("claim_id TEXT"));
        assert!(sql.contains("pending_delivery_key TEXT"));
        assert!(sql.contains("pending_unsubscribe_expires_at_ms BIGINT"));
        assert!(sql.contains("unsubscribe_nonce TEXT NOT NULL DEFAULT ''"));
        assert!(sql.contains("memory_engine_rate_limits"));
        assert!(sql.contains("rate_limit_key TEXT PRIMARY KEY"));
        assert!(sql.contains("expected_prior_schedule_state JSONB"));
        assert!(sql.contains("ON DELETE CASCADE"));
        assert!(sql.contains("memory_engine_generation_jobs"));
        assert!(sql.contains("lease_token TEXT"));
        assert!(sql.contains("reserved_cost_usd_micros BIGINT"));
        assert!(jobs_sql.contains("memory_engine_generation_jobs"));
        assert!(jobs_sql.contains("lease_expires_at_ms"));
        assert!(jobs_sql.contains("status IN ('queued', 'running', 'retry')"));
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn live_generation_jobs_are_durable_leased_and_bounded() {
        let Some(database_url) = std::env::var("MEMORY_ENGINE_POSTGRES_TEST_URL").ok() else {
            eprintln!("skipping live Postgres job test; MEMORY_ENGINE_POSTGRES_TEST_URL is unset");
            return;
        };
        let schema = format!("memory_engine_test_jobs_{}_{}", std::process::id(), NOW);
        let mut admin = crate::connect_client(&database_url).expect("connect admin postgres");
        admin
            .batch_execute(&format!(r#"CREATE SCHEMA "{schema}";"#))
            .expect("create schema");
        let scoped_url = scoped_postgres_url(&database_url, &schema);
        let result = (|| -> Result<(), PostgresStoreError> {
            let mut first = PostgresStudyStore::connect(&scoped_url)?;
            first.migrate()?;
            {
                let mut account = first.for_account(AccountScope::new("acct_jobs")?);
                account.ensure_account(NOW)?;
            }
            {
                let mut account = first.for_account(AccountScope::new("acct_other")?);
                account.ensure_account(NOW)?;
            }
            let started = first.enqueue_generation_job(
                "acct_jobs",
                "job-1",
                "source-1",
                "Title",
                "model-a",
                NOW - 2,
                2,
                4,
                100,
                86_400_000,
            )?;
            let PostgresEnqueueOutcome::Started(job) = started else {
                panic!("first enqueue must start");
            };
            assert_eq!(job.status, "queued");
            assert!(matches!(
                first.enqueue_generation_job(
                    "acct_jobs",
                    "job-2",
                    "source-1",
                    "Title",
                    "model-a",
                    NOW,
                    2,
                    4,
                    100,
                    86_400_000,
                )?,
                PostgresEnqueueOutcome::AlreadyInFlight(_)
            ));
            assert!(matches!(
                first.enqueue_generation_job(
                    "acct_jobs",
                    "job-2",
                    "source-2",
                    "Title 2",
                    "model-a",
                    NOW,
                    4,
                    4,
                    100,
                    86_400_000,
                )?,
                PostgresEnqueueOutcome::Started(_)
            ));
            assert!(matches!(
                first.enqueue_generation_job(
                    "acct_other",
                    "job-1",
                    "source-other",
                    "Other title",
                    "model-a",
                    NOW + 1,
                    4,
                    4,
                    100,
                    86_400_000,
                )?,
                PostgresEnqueueOutcome::Started(_)
            ));
            assert_eq!(
                first
                    .generation_job("acct_jobs", "job-1")?
                    .expect("first account job")
                    .source_id,
                "source-1"
            );
            assert_eq!(
                first
                    .generation_job("acct_other", "job-1")?
                    .expect("second account job")
                    .source_id,
                "source-other"
            );

            let mut restarted = PostgresStudyStore::connect(&scoped_url)?;
            restarted.migrate()?;
            assert_eq!(
                restarted.list_generation_jobs("acct_jobs", 10)?.len(),
                2,
                "both admitted account jobs must survive the restart"
            );
            let claimed = restarted
                .claim_generation_job("worker-a", NOW, 10, 0, 1, 3)?
                .expect("queued job must be claimable");
            assert_eq!(claimed.id, "job-1");
            assert_eq!(claimed.account_id, "acct_jobs");
            assert_eq!(claimed.status, "running");
            assert_eq!(claimed.attempts, 1);
            let persisted_claim = restarted
                .generation_job("acct_jobs", "job-1")?
                .expect("claimed job must be readable");
            assert_eq!(persisted_claim.lease_token, claimed.lease_token);
            let first_run_id = format!("job:{}:attempt:1", claimed.id);
            assert!(restarted.bind_generation_job_attempt_run(
                "acct_jobs",
                "job-1",
                1,
                claimed.lease_token.as_deref().expect("first lease token"),
                &first_run_id,
            )?);
            let mut account = restarted.for_account(AccountScope::new("acct_jobs")?);
            let mut completed_before_finish = generation_run(&first_run_id, &["source-1"], &[]);
            completed_before_finish.usage = Some(GenerationRunUsage {
                input_tokens: 3,
                output_tokens: 4,
                cost_usd_micros: Some(37),
                latency_ms: 5,
            });
            account.save_generation_run(&completed_before_finish)?;
            drop(account);
            assert!(restarted.renew_generation_job(
                "acct_jobs",
                "job-1",
                claimed.lease_token.as_deref().expect("first lease token"),
                NOW + 5,
                10,
            )?);
            assert!(
                !restarted.renew_generation_job(
                    "acct_jobs",
                    "job-1",
                    claimed.lease_token.as_deref().expect("first lease token"),
                    NOW + 16,
                    10,
                )?,
                "an expired owner cannot renew even before reclaim runs"
            );
            assert!(
                !restarted.finish_generation_job(
                    "acct_jobs",
                    "job-1",
                    claimed.lease_token.as_deref().expect("first lease token"),
                    NOW + 16,
                    Err("expired owner must be fenced".to_owned()),
                    3,
                    1_000,
                )?,
                "an expired owner cannot finish even before reclaim runs"
            );

            let mut after_lease = PostgresStudyStore::connect(&scoped_url)?;
            after_lease.migrate()?;
            assert!(
                after_lease
                    .claim_generation_job("worker-b", NOW + 11, 10, 0, 1, 3)?
                    .is_none(),
                "renewed lease must remain owned"
            );
            let reclaimed = after_lease
                .claim_generation_job("worker-b", NOW + 16, 10, 0, 1, 3)?
                .expect("expired lease must be reclaimed");
            assert_eq!(reclaimed.attempts, 2);
            assert!(!after_lease.finish_generation_job(
                "acct_jobs",
                "job-1",
                claimed.lease_token.as_deref().expect("first lease token"),
                NOW + 16,
                Err("stale worker must be fenced".to_owned()),
                3,
                1_000,
            )?);
            let stale_attempt = after_lease
                .generation_job_attempt(
                    "acct_jobs",
                    "job-1",
                    1,
                    claimed.lease_token.as_deref().expect("first lease token"),
                )?
                .expect("expired attempt receipt");
            assert_eq!(stale_attempt.status, "stale");
            assert_eq!(
                stale_attempt.cost_usd_micros, 37,
                "a completed provider receipt is recovered after the worker dies before finish"
            );
            assert_eq!(
                stale_attempt.generation_run_id.as_deref(),
                Some(first_run_id.as_str())
            );
            after_lease.finish_generation_job(
                "acct_jobs",
                "job-1",
                reclaimed
                    .lease_token
                    .as_deref()
                    .expect("reclaimed lease token"),
                NOW + 20,
                Err("provider unavailable".to_owned()),
                3,
                1_000,
            )?;

            let mut retry_reader = PostgresStudyStore::connect(&scoped_url)?;
            retry_reader.migrate()?;
            assert_eq!(
                retry_reader
                    .generation_job("acct_jobs", "job-1")?
                    .expect("job")
                    .status,
                "retry"
            );
            assert_eq!(
                retry_reader
                    .generation_job("acct_jobs", "job-1")?
                    .expect("retry reservation")
                    .reserved_cost_usd_micros,
                reclaimed.reserved_cost_usd_micros,
                "a failed attempt keeps its next-attempt reservation"
            );
            let retry_claim = retry_reader
                .claim_generation_job("worker-c", NOW + 2_000, 10, 0, 1, 3)?
                .expect("retry must survive restart");
            assert_eq!(retry_claim.attempts, 3);
            assert_eq!(
                retry_claim.reserved_cost_usd_micros, reclaimed.reserved_cost_usd_micros,
                "retry atomically recreates the same bounded reservation"
            );
            retry_reader.finish_generation_job(
                "acct_jobs",
                "job-1",
                retry_claim
                    .lease_token
                    .as_deref()
                    .expect("retry lease token"),
                NOW + 2_001,
                Ok((4, 25)),
                3,
                1_000,
            )?;
            let successful_attempt = retry_reader
                .generation_job_attempt(
                    "acct_jobs",
                    "job-1",
                    3,
                    retry_claim.lease_token.as_deref().expect("retry token"),
                )?
                .expect("successful attempt receipt");
            assert_eq!(successful_attempt.status, "succeeded");
            assert_eq!(successful_attempt.cost_usd_micros, 25);
            assert_eq!(
                retry_reader
                    .generation_job("acct_jobs", "job-1")?
                    .expect("job")
                    .status,
                "succeeded"
            );

            let mut account = retry_reader.for_account(AccountScope::new("acct_jobs")?);
            let mut earlier = generation_run("run-earlier", &["source-1"], &[]);
            earlier.usage = Some(GenerationRunUsage {
                input_tokens: 1,
                output_tokens: 1,
                cost_usd_micros: Some(7),
                latency_ms: 1,
            });
            account.save_generation_run(&earlier)?;
            let mut latest = generation_run("run-latest", &["source-1"], &[]);
            latest.started_at = NOW + 10;
            latest.completed_at = Some(NOW + 11);
            latest.usage = Some(GenerationRunUsage {
                input_tokens: 2,
                output_tokens: 2,
                cost_usd_micros: Some(25),
                latency_ms: 2,
            });
            account.save_generation_run(&latest)?;
            drop(account);
            assert_eq!(
                retry_reader.generation_cost_for_source("acct_jobs", "source-1")?,
                25,
                "a job receives its own latest run receipt, not cumulative source history"
            );
            assert_eq!(
                retry_reader.generation_cost_for_run("acct_jobs", "run-earlier")?,
                7
            );
            assert_eq!(
                retry_reader.generation_cost_for_run("acct_jobs", "run-latest")?,
                25
            );

            {
                let mut account = retry_reader.for_account(AccountScope::new("acct_budget")?);
                account.ensure_account(NOW)?;
            }
            retry_reader.client.borrow_mut().execute(
                "INSERT INTO memory_engine_generation_jobs
                    (account_id, job_id, source_id, title, status, model_key,
                     cost_usd_micros, created_at_ms, updated_at_ms)
                 VALUES ($1::TEXT, $2::TEXT, $3::TEXT, $4::TEXT, 'succeeded',
                         $5::TEXT, $6::BIGINT, $7::BIGINT, $7::BIGINT)",
                &[
                    &"acct_budget",
                    &"spent-job",
                    &"spent-source",
                    &"Spent",
                    &"model-budget",
                    &75_i64,
                    &NOW,
                ],
            )?;
            assert!(
                matches!(
                    retry_reader.enqueue_generation_job(
                        "acct_budget",
                        "boundary-job",
                        "boundary-source",
                        "Boundary",
                        "model-budget",
                        NOW,
                        2,
                        4,
                        100,
                        86_400_000,
                    )?,
                    PostgresEnqueueOutcome::Rejected(_)
                ),
                "spent 75 plus the proposed 50 reservation must reject a 100 budget"
            );

            let rejected = retry_reader.enqueue_generation_job(
                "acct_jobs",
                "job-3",
                "source-3",
                "Title",
                "model-a",
                NOW,
                1,
                4,
                25,
                86_400_000,
            )?;
            assert!(matches!(rejected, PostgresEnqueueOutcome::Rejected(_)));
            Ok(())
        })();
        admin
            .batch_execute(&format!(r#"DROP SCHEMA "{schema}" CASCADE;"#))
            .expect("drop schema");
        result.expect("durable generation job contract");
    }

    #[test]
    fn started_receipt_without_usage_reclaims_at_the_conservative_reservation() {
        let Some(database_url) = std::env::var("MEMORY_ENGINE_POSTGRES_TEST_URL").ok() else {
            eprintln!(
                "skipping live Postgres started-receipt regression; MEMORY_ENGINE_POSTGRES_TEST_URL is unset"
            );
            return;
        };
        let schema = format!(
            "memory_engine_test_started_receipt_{}_{}",
            std::process::id(),
            NOW
        );
        let mut admin = crate::connect_client(&database_url).expect("connect admin postgres");
        admin
            .batch_execute(&format!(r#"CREATE SCHEMA "{schema}";"#))
            .expect("create schema");
        let scoped_url = scoped_postgres_url(&database_url, &schema);
        let result = (|| -> Result<(), PostgresStoreError> {
            let mut store = PostgresStudyStore::connect(&scoped_url)?;
            store.migrate()?;
            {
                let mut account = store.for_account(AccountScope::new("acct_started")?);
                account.ensure_account(NOW)?;
            }
            store.client.borrow_mut().execute(
                "INSERT INTO memory_engine_generation_jobs
                    (account_id, job_id, source_id, title, status, model_key,
                     cost_usd_micros, created_at_ms, updated_at_ms, reserved_cost_usd_micros)
                 VALUES ($1::TEXT, $2::TEXT, $3::TEXT, $4::TEXT, 'queued',
                         $5::TEXT, 0::BIGINT, $6::BIGINT, $6::BIGINT, $7::BIGINT)",
                &[
                    &"acct_started",
                    &"job-started",
                    &"source-started",
                    &"Started receipt",
                    &"model-started",
                    &(NOW - 86_400_000),
                    &50_i64,
                ],
            )?;
            let claimed = store
                .claim_generation_job("worker-a", NOW, 10, 0, 1, 3)?
                .expect("claim started-receipt job");
            assert_eq!(claimed.attempts, 1);
            let run_id = format!("job:{}:attempt:{}", claimed.id, claimed.attempts);
            assert!(store.bind_generation_job_attempt_run(
                "acct_started",
                "job-started",
                1,
                claimed.lease_token.as_deref().expect("lease token"),
                &run_id,
            )?);
            let mut account = store.for_account(AccountScope::new("acct_started")?);
            let started_only = generation_run(&run_id, &["source-started"], &[]);
            account.save_generation_run(&started_only)?;
            drop(account);
            let reclaimed = store
                .claim_generation_job("worker-b", NOW + 16, 10, 0, 1, 3)?
                .expect("expired started receipt job must be reclaimable");
            assert_eq!(reclaimed.attempts, 2);
            let stale_attempt = store
                .generation_job_attempt(
                    "acct_started",
                    "job-started",
                    1,
                    claimed.lease_token.as_deref().expect("lease token"),
                )?
                .expect("stale started attempt");
            assert_eq!(stale_attempt.status, "stale");
            assert_eq!(
                stale_attempt.cost_usd_micros, claimed.reserved_cost_usd_micros,
                "missing usage must fall back to the conservative reservation, never zero"
            );
            Ok(())
        })();
        admin
            .batch_execute(&format!(r#"DROP SCHEMA "{schema}" CASCADE;"#))
            .expect("drop schema");
        result.expect("started receipt regression");
    }

    #[test]
    fn legacy_running_v2_jobs_with_null_lease_expiry_are_reclaimable_after_upgrade() {
        let Some(database_url) = std::env::var("MEMORY_ENGINE_POSTGRES_TEST_URL").ok() else {
            eprintln!(
                "skipping live Postgres legacy-running upgrade regression; MEMORY_ENGINE_POSTGRES_TEST_URL is unset"
            );
            return;
        };
        let schema = format!(
            "memory_engine_test_v2_running_{}_{}",
            std::process::id(),
            NOW
        );
        let mut admin = crate::connect_client(&database_url).expect("connect admin postgres");
        admin
            .batch_execute(&format!(r#"CREATE SCHEMA "{schema}";"#))
            .expect("create schema");
        let scoped_url = scoped_postgres_url(&database_url, &schema);
        let result = (|| -> Result<(), PostgresStoreError> {
            let mut store = PostgresStudyStore::connect(&scoped_url)?;
            store
                .client
                .borrow_mut()
                .batch_execute(super::MIGRATION_TABLE_SQL)?;
            store
                .client
                .borrow_mut()
                .batch_execute(super::BASE_MIGRATION_SQL)?;
            store.client.borrow_mut().batch_execute(
                r"
                CREATE TABLE memory_engine_generation_jobs (
                    account_id TEXT NOT NULL REFERENCES memory_engine_accounts(account_id) ON DELETE CASCADE,
                    job_id TEXT NOT NULL, source_id TEXT NOT NULL, title TEXT NOT NULL,
                    status TEXT NOT NULL, card_count INTEGER NOT NULL DEFAULT 0,
                    attempts INTEGER NOT NULL DEFAULT 0, error TEXT, model_key TEXT NOT NULL,
                    cost_usd_micros BIGINT NOT NULL DEFAULT 0, created_at_ms BIGINT NOT NULL,
                    updated_at_ms BIGINT NOT NULL, retry_at_ms BIGINT, lease_owner TEXT,
                    lease_expires_at_ms BIGINT, PRIMARY KEY (account_id, job_id)
                );
                INSERT INTO memory_engine_accounts (account_id, created_at_ms)
                    VALUES ('upgrade-account', 1);
                INSERT INTO memory_engine_generation_jobs
                    (account_id, job_id, source_id, title, status, model_key,
                     created_at_ms, updated_at_ms, attempts, lease_expires_at_ms)
                VALUES ('upgrade-account', 'upgrade-job', 'upgrade-source', 'Upgrade source',
                        'running', 'model-a', 1, 1, 1, NULL);
                INSERT INTO memory_engine_schema_migrations (version, applied_at_ms)
                    VALUES (1, 1), (2, 2);
                ",
            )?;
            store.migrate()?;
            let claimed = store
                .claim_generation_job("worker-a", NOW + 16, 10, 0, 1, 3)?
                .expect("legacy running row must be reclaimable");
            assert_eq!(claimed.id, "upgrade-job");
            assert_eq!(claimed.attempts, 2);
            assert!(
                claimed.lease_token.is_some(),
                "reclaimed job must receive a safe lease token"
            );
            let attempt = store
                .generation_job_attempt(
                    "upgrade-account",
                    "upgrade-job",
                    2,
                    claimed.lease_token.as_deref().expect("lease token"),
                )?
                .expect("reclaimed attempt");
            assert_eq!(attempt.status, "running");
            assert_eq!(
                attempt.reservation_cost_usd_micros,
                attempt.reserved_cost_usd_micros
            );
            Ok(())
        })();
        admin
            .batch_execute(&format!(r#"DROP SCHEMA "{schema}" CASCADE;"#))
            .expect("drop schema");
        result.expect("legacy running v2 upgrade");
    }

    #[test]
    fn return_notification_claim_sql_declares_i64_parameters_as_bigint() {
        for parameter in [
            "$2::BIGINT",
            "$3::BIGINT",
            "$5::BIGINT",
            "$7::BIGINT",
            "$10::BIGINT",
        ] {
            assert!(
                CLAIM_RETURN_NOTIFICATION_SQL.contains(parameter),
                "claim SQL must explicitly bind {parameter} as BIGINT"
            );
        }
    }

    #[test]
    fn generation_lease_sql_declares_bigint_arithmetic_parameters() {
        for parameter in ["$1::TEXT", "$2::TEXT", "$3::TEXT"] {
            assert!(
                RENEW_GENERATION_JOB_SQL.contains(parameter),
                "renew SQL must explicitly bind {parameter} as TEXT"
            );
        }
        assert!(RENEW_GENERATION_JOB_SQL.contains("$4::BIGINT + $5::BIGINT"));
        assert!(RENEW_GENERATION_JOB_SQL.contains("updated_at_ms = $4::BIGINT"));
        assert!(RENEW_GENERATION_JOB_SQL.contains("lease_expires_at_ms > $4::BIGINT"));
    }

    #[test]
    fn generation_finish_sql_declares_case_and_nullable_parameter_types() {
        for fragment in [
            "$1::BOOLEAN",
            "$2::INTEGER",
            "$3::BIGINT",
            "$4::TEXT",
            "$5::BIGINT",
            "$6::BIGINT",
            "$7::TEXT",
            "$8::TEXT",
            "$9::INTEGER",
            "$10::TEXT",
            "$11::BIGINT",
            "NULL::BIGINT",
        ] {
            assert!(
                FINISH_GENERATION_JOB_SQL.contains(fragment),
                "finish SQL must explicitly type {fragment}"
            );
        }
        for fragment in [
            "$1::TEXT",
            "$2::BIGINT",
            "$3::TEXT",
            "$4::BIGINT",
            "$5::TEXT",
            "$6::TEXT",
            "$7::INTEGER",
            "$8::TEXT",
            "status = 'running'",
        ] {
            assert!(
                super::FINISH_GENERATION_JOB_ATTEMPT_SQL.contains(fragment),
                "attempt receipt SQL must explicitly type {fragment}"
            );
        }
    }

    #[test]
    fn generation_claim_sql_treats_null_lease_expiry_as_reclaimable() {
        for fragment in [
            "lease_expires_at_ms IS NULL",
            "OR lease_expires_at_ms + $3::BIGINT < $1::BIGINT",
            "OR job.lease_expires_at_ms + $3::BIGINT < $1::BIGINT",
        ] {
            assert!(
                claim_generation_job_sql_contains(fragment),
                "claim SQL must treat missing lease expiry as reclaimable: {fragment}"
            );
        }
    }

    fn claim_generation_job_sql_contains(fragment: &str) -> bool {
        let claim_sql = r"
            UPDATE memory_engine_generation_job_attempts attempt
             SET status = 'stale',
                 cost_usd_micros = CASE
                     WHEN attempt.generation_run_id IS NOT NULL AND EXISTS (
                         SELECT 1 FROM memory_engine_generation_runs run
                         WHERE run.account_id = attempt.account_id
                           AND run.generation_run_id = attempt.generation_run_id
                     ) THEN COALESCE((
                         SELECT (run->'usage'->>'costUsdMicros')::BIGINT
                         FROM memory_engine_generation_runs run
                         WHERE run.account_id = attempt.account_id
                           AND run.generation_run_id = attempt.generation_run_id
                     ), GREATEST(attempt.reservation_cost_usd_micros,
                                 attempt.reserved_cost_usd_micros))
                     ELSE GREATEST(attempt.cost_usd_micros, attempt.reserved_cost_usd_micros)
                 END,
                 reserved_cost_usd_micros = 0::BIGINT,
                 error = 'Lease expired before the provider attempt completed.',
                 completed_at_ms = $1::BIGINT, updated_at_ms = $1::BIGINT
             FROM memory_engine_generation_jobs job
             WHERE job.account_id = attempt.account_id AND job.job_id = attempt.job_id
               AND job.status = 'running' AND job.attempts = attempt.attempt
               AND job.lease_token = attempt.lease_token
               AND (job.lease_expires_at_ms IS NULL
                    OR job.lease_expires_at_ms + $2::BIGINT < $1::BIGINT)
               AND attempt.status = 'running'";
        let claim_sql_two = r"
            UPDATE memory_engine_generation_jobs
             SET status = 'failed', error = 'Maximum generation attempts exhausted.',
                 updated_at_ms = $1::BIGINT, lease_owner = NULL::TEXT,
                 lease_expires_at_ms = NULL::BIGINT, lease_token = NULL::TEXT,
                 reserved_cost_usd_micros = 0::BIGINT
             WHERE status IN ('running', 'retry') AND attempts >= $2::INTEGER
               AND (status = 'retry'
                    OR lease_expires_at_ms IS NULL
                    OR lease_expires_at_ms + $3::BIGINT < $1::BIGINT)";
        let claim_sql_three = r"
            UPDATE memory_engine_generation_jobs
             SET status = 'retry', error = 'Lease expired; recovery is retrying this job.',
                 retry_at_ms = $1::BIGINT, updated_at_ms = $1::BIGINT,
                 lease_owner = NULL::TEXT, lease_expires_at_ms = NULL::BIGINT,
                 lease_token = NULL::TEXT
             WHERE status = 'running' AND attempts < $2::INTEGER
               AND (lease_expires_at_ms IS NULL
                    OR lease_expires_at_ms + $3::BIGINT < $1::BIGINT)";
        let claim_sql_four = r"
                WHERE (job.status = 'queued' OR (job.status = 'retry' AND job.retry_at_ms <= $1::BIGINT)
                       OR (job.status = 'running'
                           AND (job.lease_expires_at_ms IS NULL
                                OR job.lease_expires_at_ms + $3::BIGINT < $1::BIGINT)))";
        [claim_sql, claim_sql_two, claim_sql_three, claim_sql_four]
            .iter()
            .any(|sql| sql.contains(fragment))
    }

    #[test]
    fn deployed_v2_generation_jobs_upgrade_preserves_rows_and_adds_attempt_ledger() {
        let Some(database_url) = std::env::var("MEMORY_ENGINE_POSTGRES_TEST_URL").ok() else {
            eprintln!("skipping live Postgres migration upgrade test; MEMORY_ENGINE_POSTGRES_TEST_URL is unset");
            return;
        };
        let schema = format!(
            "memory_engine_test_v2_upgrade_{}_{}",
            std::process::id(),
            NOW
        );
        let mut admin = crate::connect_client(&database_url).expect("connect admin postgres");
        admin
            .batch_execute(&format!(r#"CREATE SCHEMA "{schema}";"#))
            .expect("create schema");
        let scoped_url = scoped_postgres_url(&database_url, &schema);
        let result = (|| -> Result<(), PostgresStoreError> {
            let mut store = PostgresStudyStore::connect(&scoped_url)?;
            store
                .client
                .borrow_mut()
                .batch_execute(super::MIGRATION_TABLE_SQL)?;
            store
                .client
                .borrow_mut()
                .batch_execute(super::BASE_MIGRATION_SQL)?;
            store.client.borrow_mut().batch_execute(
                r"
                CREATE TABLE memory_engine_generation_jobs (
                    account_id TEXT NOT NULL REFERENCES memory_engine_accounts(account_id) ON DELETE CASCADE,
                    job_id TEXT NOT NULL, source_id TEXT NOT NULL, title TEXT NOT NULL,
                    status TEXT NOT NULL, card_count INTEGER NOT NULL DEFAULT 0,
                    attempts INTEGER NOT NULL DEFAULT 0, error TEXT, model_key TEXT NOT NULL,
                    cost_usd_micros BIGINT NOT NULL DEFAULT 0, created_at_ms BIGINT NOT NULL,
                    updated_at_ms BIGINT NOT NULL, retry_at_ms BIGINT, lease_owner TEXT,
                    lease_expires_at_ms BIGINT, PRIMARY KEY (account_id, job_id)
                );
                INSERT INTO memory_engine_accounts (account_id, created_at_ms) VALUES ('upgrade-account', 1);
                INSERT INTO memory_engine_generation_jobs
                    (account_id, job_id, source_id, title, status, model_key, created_at_ms, updated_at_ms)
                VALUES ('upgrade-account', 'upgrade-job', 'upgrade-source', 'preserved', 'queued', 'model-a', 1, 1);
                INSERT INTO memory_engine_schema_migrations (version, applied_at_ms) VALUES (1, 1), (2, 2);
                ",
            )?;
            store.migrate()?;
            let job = store
                .generation_job("upgrade-account", "upgrade-job")?
                .expect("v2 row survives upgrade");
            assert_eq!(job.title, "preserved");
            assert_eq!(job.reserved_cost_usd_micros, 0);
            let attempts: i64 = store
                .client
                .borrow_mut()
                .query_one(
                    "SELECT COUNT(*) FROM information_schema.tables
                     WHERE table_schema = current_schema()
                       AND table_name = 'memory_engine_generation_job_attempts'",
                    &[],
                )?
                .get(0);
            assert_eq!(attempts, 1);
            Ok(())
        })();
        admin
            .batch_execute(&format!(r#"DROP SCHEMA "{schema}" CASCADE;"#))
            .expect("drop schema");
        result.expect("v2 generation job upgrade");
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

        let scoped_url = scoped_postgres_url(&database_url, &schema);
        let result = run_live_postgres_store_contract(&scoped_url);
        admin
            .batch_execute(&format!(r#"DROP SCHEMA "{schema}" CASCADE;"#))
            .expect("drop schema");
        result.expect("live postgres store contract");
    }

    #[test]
    // This intentionally keeps the two database race scenarios together so
    // their shared setup and cleanup are visible in one acceptance oracle.
    #[allow(clippy::too_many_lines)]
    fn live_postgres_feedback_concurrency_is_idempotent_and_single_head() {
        let Some(database_url) = std::env::var("MEMORY_ENGINE_POSTGRES_TEST_URL").ok() else {
            eprintln!("skipping live Postgres test; MEMORY_ENGINE_POSTGRES_TEST_URL is unset");
            return;
        };
        let schema = format!(
            "memory_engine_test_feedback_race_{}_{}",
            std::process::id(),
            NOW + 1
        );
        let mut admin = crate::connect_client(&database_url).expect("connect admin postgres");
        admin
            .batch_execute(&format!(r#"CREATE SCHEMA "{schema}";"#))
            .expect("create schema");
        let scoped_url = scoped_postgres_url(&database_url, &schema);
        let result = (|| -> Result<(), super::PostgresStoreError> {
            let mut setup = super::PostgresStudyStore::connect(&scoped_url)?;
            setup.migrate()?;
            let unit = ReviewUnitId::new("unit-live-feedback-race");
            let source = source_document("source-live-feedback-race");
            let reference = reference_span("reference-live-feedback-race", &source.id);
            let draft = accepted_draft(
                "draft-live-feedback-race",
                &unit,
                &[&source.id],
                &[&reference.id],
                Some("run-live-feedback-race"),
            );
            let run = generation_run("run-live-feedback-race", &[&source.id], &[&draft.id]);
            let record = review_unit(&draft);
            {
                let mut account = setup.for_account(AccountScope::new("acct-feedback-race")?);
                account.ensure_account(NOW)?;
                account.save_source_document(&source)?;
                account.save_reference_span(&reference)?;
                account.save_generation_run(&run)?;
                account.save_generated_prompt_draft(&draft)?;
                account.save_review_unit(&record)?;
                record_content_feedback(
                    &mut account,
                    RecordContentFeedbackCommand {
                        feedback_id: "feedback-live-race-root".to_owned(),
                        review_unit_id: unit.clone(),
                        verdict: ContentFeedbackVerdict::Dropped,
                        rationale: None,
                        account_id: "acct-feedback-race".to_owned(),
                        occurred_at: NOW,
                        supersedes_id: None,
                    },
                )
                .map_err(|error| super::PostgresStoreError::StudySession(error.to_string()))?;
            }
            drop(setup);

            let barrier = Arc::new(Barrier::new(2));
            let mut same_id_handles = Vec::new();
            for _ in 0..2 {
                let barrier = Arc::clone(&barrier);
                let url = scoped_url.clone();
                same_id_handles.push(std::thread::spawn(move || {
                    let mut store = super::PostgresStudyStore::connect(&url).expect("race connect");
                    let mut account =
                        store.for_account(AccountScope::new("acct-feedback-race").expect("scope"));
                    barrier.wait();
                    record_content_feedback(
                        &mut account,
                        RecordContentFeedbackCommand {
                            feedback_id: "feedback-live-race-same-id".to_owned(),
                            review_unit_id: ReviewUnitId::new("unit-live-feedback-race"),
                            verdict: ContentFeedbackVerdict::Kept,
                            rationale: Some("same payload".to_owned()),
                            account_id: "acct-feedback-race".to_owned(),
                            occurred_at: NOW + 1_000,
                            supersedes_id: Some("feedback-live-race-root".to_owned()),
                        },
                    )
                }));
            }
            let same_id_results = same_id_handles
                .into_iter()
                .map(|handle| handle.join().expect("same-id worker"))
                .collect::<Vec<_>>();
            assert!(
                same_id_results.iter().all(Result::is_ok),
                "same-id replay must be idempotent: {same_id_results:?}"
            );

            let stale_unit = ReviewUnitId::new("unit-live-feedback-stale");
            let mut stale_record = record.clone();
            stale_record.review_unit_id = stale_unit.clone();
            stale_record.queue.review_unit_id = stale_unit.clone();
            {
                let mut store = super::PostgresStudyStore::connect(&scoped_url)?;
                let mut account = store.for_account(AccountScope::new("acct-feedback-race")?);
                account.save_review_unit(&stale_record)?;
                record_content_feedback(
                    &mut account,
                    RecordContentFeedbackCommand {
                        feedback_id: "feedback-live-stale-root".to_owned(),
                        review_unit_id: stale_unit.clone(),
                        verdict: ContentFeedbackVerdict::Dropped,
                        rationale: None,
                        account_id: "acct-feedback-race".to_owned(),
                        occurred_at: NOW,
                        supersedes_id: None,
                    },
                )
                .map_err(|error| super::PostgresStoreError::StudySession(error.to_string()))?;
            }
            let barrier = Arc::new(Barrier::new(2));
            let mut stale_handles = Vec::new();
            for id in ["feedback-live-stale-a", "feedback-live-stale-b"] {
                let barrier = Arc::clone(&barrier);
                let url = scoped_url.clone();
                stale_handles.push(std::thread::spawn(move || {
                    let mut store =
                        super::PostgresStudyStore::connect(&url).expect("stale connect");
                    let mut account =
                        store.for_account(AccountScope::new("acct-feedback-race").expect("scope"));
                    barrier.wait();
                    record_content_feedback(
                        &mut account,
                        RecordContentFeedbackCommand {
                            feedback_id: id.to_owned(),
                            review_unit_id: ReviewUnitId::new("unit-live-feedback-stale"),
                            verdict: ContentFeedbackVerdict::Kept,
                            rationale: None,
                            account_id: "acct-feedback-race".to_owned(),
                            occurred_at: NOW + 2_000,
                            supersedes_id: Some("feedback-live-stale-root".to_owned()),
                        },
                    )
                }));
            }
            let stale_results = stale_handles
                .into_iter()
                .map(|handle| handle.join().expect("stale worker"))
                .collect::<Vec<_>>();
            assert_eq!(
                stale_results.iter().filter(|result| result.is_ok()).count(),
                1,
                "exactly one concurrent child may win the locked head: {stale_results:?}"
            );
            assert_eq!(
                stale_results
                    .iter()
                    .filter(|result| matches!(
                        result,
                        Err(ContentFeedbackError::Store(
                            super::PostgresStoreError::FeedbackSupersedesStale { .. }
                        ))
                    ))
                    .count(),
                1,
                "the losing concurrent child must be a typed stale conflict: {stale_results:?}"
            );
            Ok(())
        })();
        admin
            .batch_execute(&format!(r#"DROP SCHEMA "{schema}" CASCADE;"#))
            .expect("drop schema");
        result.expect("live Postgres feedback race contract");
    }

    #[test]
    fn live_postgres_rate_limits_are_atomic_for_absent_rows() {
        let Some(database_url) = std::env::var("MEMORY_ENGINE_POSTGRES_TEST_URL").ok() else {
            eprintln!("skipping live Postgres test; MEMORY_ENGINE_POSTGRES_TEST_URL is unset");
            return;
        };
        let schema = format!(
            "memory_engine_test_rate_limit_{}_{}",
            std::process::id(),
            NOW
        );
        let mut admin = crate::connect_client(&database_url).expect("connect admin postgres");
        admin
            .batch_execute(&format!(r#"CREATE SCHEMA "{schema}";"#))
            .expect("create schema");

        let scoped_url = scoped_postgres_url(&database_url, &schema);
        let result = run_live_postgres_rate_limit_contract(&scoped_url);
        admin
            .batch_execute(&format!(r#"DROP SCHEMA "{schema}" CASCADE;"#))
            .expect("drop schema");
        result.expect("live postgres rate limit contract");
    }

    #[test]
    fn live_postgres_return_notification_claim_is_atomic_and_fenced() {
        let Some(database_url) = std::env::var("MEMORY_ENGINE_POSTGRES_TEST_URL").ok() else {
            eprintln!("skipping live Postgres test; MEMORY_ENGINE_POSTGRES_TEST_URL is unset");
            return;
        };
        let schema = format!(
            "memory_engine_test_return_claim_{}_{}",
            std::process::id(),
            NOW
        );
        let mut admin = crate::connect_client(&database_url).expect("connect admin postgres");
        admin
            .batch_execute(&format!(r#"CREATE SCHEMA "{schema}";"#))
            .expect("create schema");
        let scoped_url = scoped_postgres_url(&database_url, &schema);
        let result = (|| -> Result<(), super::PostgresStoreError> {
            let mut setup = super::PostgresStudyStore::connect(&scoped_url)?;
            setup.migrate()?;
            {
                let scope = super::AccountScope::new("acct-claim")?;
                let mut account = setup.for_account(scope);
                account.ensure_account(NOW)?;
            }
            setup.save_return_notification_preference(
                "acct-claim",
                "claim@example.com",
                true,
                None,
                NOW,
                "claim-nonce",
            )?;
            drop(setup);

            let barrier = Arc::new(Barrier::new(16));
            let workers = (0..16)
                .map(|index| {
                    let barrier = Arc::clone(&barrier);
                    let scoped_url = scoped_url.clone();
                    std::thread::spawn(move || {
                        let mut store = super::PostgresStudyStore::connect(&scoped_url)
                            .expect("claim connection");
                        barrier.wait();
                        store
                            .claim_return_notification(&super::ReturnNotificationClaimRequest {
                                account_id: "acct-claim".to_owned(),
                                now_ms: NOW,
                                due_count: 4,
                                force_confirmation: true,
                                interval_ms: 86_400_000,
                                claim_id: format!("claim-{index}"),
                                delivery_key: "delivery-claim".to_owned(),
                                claim_expires_at_ms: NOW + 300_000,
                                unsubscribe_nonce: "claim-nonce".to_owned(),
                                unsubscribe_expires_at_ms: NOW + 604_800_000,
                            })
                            .expect("claim")
                    })
                })
                .collect::<Vec<_>>();
            let claims = workers
                .into_iter()
                .filter_map(|worker| worker.join().expect("claim worker"))
                .collect::<Vec<_>>();
            assert_eq!(claims.len(), 1, "exactly one Postgres worker may claim");
            let winner = &claims[0];
            assert_eq!(winner.unsubscribe_expires_at_ms, NOW + 604_800_000);
            let mut finalize = super::PostgresStudyStore::connect(&scoped_url)?;
            assert!(finalize.complete_return_notification(
                "acct-claim",
                &winner.claim_id,
                NOW + 1,
            )?);
            assert!(!finalize.complete_return_notification(
                "acct-claim",
                &winner.claim_id,
                NOW + 2,
            )?);

            Ok(())
        })();
        admin
            .batch_execute(&format!(r#"DROP SCHEMA "{schema}" CASCADE;"#))
            .expect("drop schema");
        result.expect("live Postgres return claim contract");
    }

    #[test]
    fn live_postgres_return_notification_retry_persists_expiry_across_stale_claims() {
        let Some(database_url) = std::env::var("MEMORY_ENGINE_POSTGRES_TEST_URL").ok() else {
            eprintln!("skipping live Postgres test; MEMORY_ENGINE_POSTGRES_TEST_URL is unset");
            return;
        };
        let schema = format!(
            "memory_engine_test_return_retry_{}_{}",
            std::process::id(),
            NOW
        );
        let mut admin = super::connect_client(&database_url).expect("connect admin postgres");
        admin
            .batch_execute(&format!(r#"CREATE SCHEMA "{schema}";"#))
            .expect("create schema");
        let scoped_url = scoped_postgres_url(&database_url, &schema);
        let result = (|| -> Result<(), super::PostgresStoreError> {
            let mut retry = super::PostgresStudyStore::connect(&scoped_url)?;
            retry.migrate()?;
            ensure_live_account(&mut retry, "acct-claim-retry")?;
            retry.save_return_notification_preference(
                "acct-claim-retry",
                "retry@example.com",
                true,
                None,
                NOW,
                "retry-nonce",
            )?;
            ensure_live_account(&mut retry, "acct-claim-retry-ready")?;
            retry.save_return_notification_preference(
                "acct-claim-retry-ready",
                "ready@example.com",
                true,
                None,
                NOW,
                "ready-nonce",
            )?;
            let first = retry
                .claim_return_notification(&super::ReturnNotificationClaimRequest {
                    account_id: "acct-claim-retry".to_owned(),
                    now_ms: NOW,
                    due_count: 2,
                    force_confirmation: true,
                    interval_ms: 86_400_000,
                    claim_id: "retry-stale-1".to_owned(),
                    delivery_key: "retry-delivery-1".to_owned(),
                    claim_expires_at_ms: NOW + 100,
                    unsubscribe_nonce: "retry-nonce-request".to_owned(),
                    unsubscribe_expires_at_ms: NOW + 604_800_000,
                })?
                .expect("first retry claim");
            retry.save_return_notification_preference(
                "acct-claim-retry",
                "retry@example.com",
                true,
                None,
                NOW + 1,
                "retry-nonce-request-rotated",
            )?;
            assert_retry_backoff_gate(&mut retry, "acct-claim-retry", &first.claim_id)?;
            assert_ready_retry_account_is_not_starved(&mut retry)?;
            let second = retry
                .claim_return_notification(&super::ReturnNotificationClaimRequest {
                    account_id: "acct-claim-retry".to_owned(),
                    now_ms: NOW + 60_003,
                    due_count: 1,
                    force_confirmation: false,
                    interval_ms: 86_400_000,
                    claim_id: "retry-stale-2".to_owned(),
                    delivery_key: "retry-delivery-2".to_owned(),
                    claim_expires_at_ms: NOW + 60_303,
                    unsubscribe_nonce: "retry-nonce-request-2".to_owned(),
                    unsubscribe_expires_at_ms: NOW + 604_860_003,
                })?
                .expect("stale retry claim");
            assert_eq!(second.delivery_key, first.delivery_key);
            assert_eq!(second.unsubscribe_nonce, first.unsubscribe_nonce);
            assert_eq!(
                second.unsubscribe_expires_at_ms,
                first.unsubscribe_expires_at_ms
            );
            assert!(!retry.complete_return_notification(
                "acct-claim-retry",
                &first.claim_id,
                NOW + 60_004,
            )?);
            assert!(retry.complete_return_notification(
                "acct-claim-retry",
                &second.claim_id,
                NOW + 60_005,
            )?);
            Ok(())
        })();
        admin
            .batch_execute(&format!(r#"DROP SCHEMA "{schema}" CASCADE;"#))
            .expect("drop schema");
        result.expect("live Postgres return retry contract");
    }

    fn assert_retry_backoff_gate(
        retry: &mut super::PostgresStudyStore,
        account_id: &str,
        claim_id: &str,
    ) -> Result<(), super::PostgresStoreError> {
        retry.release_return_notification(account_id, claim_id, NOW + 2)?;
        assert!(retry
            .claim_return_notification(&super::ReturnNotificationClaimRequest {
                account_id: account_id.to_owned(),
                now_ms: NOW + 3,
                due_count: 1,
                force_confirmation: false,
                interval_ms: 86_400_000,
                claim_id: "retry-too-soon".to_owned(),
                delivery_key: "retry-delivery-too-soon".to_owned(),
                claim_expires_at_ms: NOW + 303,
                unsubscribe_nonce: "retry-nonce-too-soon".to_owned(),
                unsubscribe_expires_at_ms: NOW + 604_800_003,
            })?
            .is_none());
        Ok(())
    }

    fn ensure_live_account(
        store: &mut super::PostgresStudyStore,
        account_id: &str,
    ) -> Result<(), super::PostgresStoreError> {
        let scope = super::AccountScope::new(account_id)?;
        let mut account = store.for_account(scope);
        account.ensure_account(NOW)
    }

    fn assert_ready_retry_account_is_not_starved(
        retry: &mut super::PostgresStudyStore,
    ) -> Result<(), super::PostgresStoreError> {
        let eligible = retry.enabled_return_notification_accounts(1, NOW + 3, 86_400_000)?;
        assert_eq!(
            eligible
                .iter()
                .map(|account| account.account_id.as_str())
                .collect::<Vec<_>>(),
            vec!["acct-claim-retry-ready"],
            "a future retry must not consume the scheduler batch before a ready account"
        );
        Ok(())
    }

    #[test]
    fn live_postgres_return_notification_unsubscribe_nonce_race_is_atomic() {
        let Some(database_url) = std::env::var("MEMORY_ENGINE_POSTGRES_TEST_URL").ok() else {
            eprintln!("skipping live Postgres test; MEMORY_ENGINE_POSTGRES_TEST_URL is unset");
            return;
        };
        let schema = format!(
            "memory_engine_test_return_unsubscribe_race_{}_{}",
            std::process::id(),
            NOW
        );
        let mut admin = crate::connect_client(&database_url).expect("connect admin postgres");
        admin
            .batch_execute(&format!(r#"CREATE SCHEMA "{schema}";"#))
            .expect("create schema");
        let scoped_url = scoped_postgres_url(&database_url, &schema);
        let result = (|| -> Result<(), super::PostgresStoreError> {
            let mut setup = super::PostgresStudyStore::connect(&scoped_url)?;
            setup.migrate()?;
            {
                let scope = super::AccountScope::new("acct-unsubscribe-race")?;
                let mut account = setup.for_account(scope);
                account.ensure_account(NOW)?;
            }
            setup.save_return_notification_preference(
                "acct-unsubscribe-race",
                "race@example.com",
                true,
                None,
                NOW,
                "nonce-before",
            )?;
            drop(setup);

            let barrier = Arc::new(Barrier::new(2));
            let reenable_barrier = Arc::clone(&barrier);
            let reenable_url = scoped_url.clone();
            let reenable = std::thread::spawn(move || {
                let mut store =
                    super::PostgresStudyStore::connect(&reenable_url).expect("reenable connection");
                reenable_barrier.wait();
                store
                    .save_return_notification_preference(
                        "acct-unsubscribe-race",
                        "race@example.com",
                        true,
                        None,
                        NOW + 2,
                        "nonce-reenabled",
                    )
                    .expect("reenable preference");
            });
            let stale_barrier = Arc::clone(&barrier);
            let stale_url = scoped_url.clone();
            let stale = std::thread::spawn(move || {
                let mut store =
                    super::PostgresStudyStore::connect(&stale_url).expect("stale connection");
                stale_barrier.wait();
                store
                    .disable_return_notification(
                        "acct-unsubscribe-race",
                        "race@example.com",
                        "nonce-before",
                        "nonce-stale",
                        NOW + 2,
                    )
                    .expect("stale token operation")
            });
            reenable.join().expect("reenable worker");
            let stale_changed = stale.join().expect("stale worker");

            let mut verify = super::PostgresStudyStore::connect(&scoped_url)?;
            let preference = verify
                .return_notification_preference("acct-unsubscribe-race")?
                .expect("race preference");
            assert!(preference.enabled);
            assert_eq!(preference.unsubscribe_nonce, "nonce-reenabled");
            let _ = stale_changed;
            assert!(verify.disable_return_notification(
                "acct-unsubscribe-race",
                "race@example.com",
                "nonce-reenabled",
                "nonce-after",
                NOW + 3,
            )?);
            assert!(!verify.disable_return_notification(
                "acct-unsubscribe-race",
                "race@example.com",
                "nonce-before",
                "nonce-replayed",
                NOW + 4,
            )?);
            Ok(())
        })();
        admin
            .batch_execute(&format!(r#"DROP SCHEMA "{schema}" CASCADE;"#))
            .expect("drop schema");
        result.expect("live Postgres return unsubscribe race contract");
    }

    fn run_live_postgres_store_contract(database_url: &str) -> Result<(), PostgresStoreError> {
        let mut store = PostgresStudyStore::connect(database_url)?;
        store.migrate()?;

        run_low_level_postgres_store_contract(&mut store)?;
        run_postgres_study_session_contract(&mut store)?;
        run_postgres_concept_snooze_contract(&mut store)?;

        Ok(())
    }

    fn scoped_postgres_url(database_url: &str, schema: &str) -> String {
        format!(
            "{}{}options=-csearch_path%3D{}",
            database_url,
            if database_url.contains('?') { '&' } else { '?' },
            schema
        )
    }

    fn run_live_postgres_rate_limit_contract(database_url: &str) -> Result<(), PostgresStoreError> {
        const CONCURRENT_ATTEMPTS: usize = 12;
        const MAX_ATTEMPTS: i32 = 5;

        let mut store = PostgresStudyStore::connect(database_url)?;
        store.migrate()?;

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(CONCURRENT_ATTEMPTS));
        let database_url = database_url.to_owned();
        let mut handles = Vec::with_capacity(CONCURRENT_ATTEMPTS);
        for attempt in 0..CONCURRENT_ATTEMPTS {
            let barrier = std::sync::Arc::clone(&barrier);
            let database_url = database_url.clone();
            handles.push(std::thread::spawn(move || -> Result<bool, String> {
                let keys = vec![
                    "app-account-email:race@example.com".to_owned(),
                    "app-account-ip:203.0.113.11".to_owned(),
                ];
                let mut store = PostgresStudyStore::connect(&database_url)
                    .map_err(|error| error.to_string())?;
                barrier.wait();
                store
                    .record_rate_limit_attempts(
                        &keys,
                        NOW + i64::try_from(attempt).expect("attempt fits i64"),
                        900_000,
                        MAX_ATTEMPTS,
                    )
                    .map_err(|error| error.to_string())
            }));
        }

        let accepted = handles
            .into_iter()
            .map(|handle| handle.join().expect("rate limit worker did not panic"))
            .collect::<Result<Vec<_>, _>>()
            .map_err(PostgresStoreError::StudySession)?;
        assert_eq!(
            accepted.iter().filter(|accepted| **accepted).count(),
            usize::try_from(MAX_ATTEMPTS).expect("max attempts fits usize")
        );

        let keys = vec![
            "app-account-email:race@example.com".to_owned(),
            "app-account-ip:203.0.113.11".to_owned(),
        ];
        assert!(!store.record_rate_limit_attempts(&keys, NOW + 60_000, 900_000, MAX_ATTEMPTS)?);

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
        let base_review_unit = review_unit(&draft);
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
            account_a.save_review_unit(&base_review_unit)?;
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

            record_live_content_feedback(&mut account_a, &review_unit_id)?;
            let mut expected_attempts = vec![attempt.clone()];
            for index in 0..11 {
                let tied_attempt =
                    service_attempt(&review_unit_id, &format!("same-ms-live-{index}"), NOW);
                account_a.record_attempt(tied_attempt.clone())?;
                expected_attempts.push(tied_attempt);
            }

            let snapshot = account_a.snapshot()?;
            assert_eq!(snapshot.source_documents, vec![source.clone()]);
            assert_eq!(snapshot.reference_spans, vec![reference.clone()]);
            assert_eq!(snapshot.generation_runs, vec![run.clone()]);
            assert_eq!(snapshot.generated_prompt_drafts, vec![draft.clone()]);
            assert_eq!(snapshot.review_units, vec![base_review_unit.clone()]);
            assert_eq!(snapshot.schedules.len(), 1);
            assert_eq!(snapshot.schedules[0].review_unit_id, review_unit_id);
            assert_eq!(snapshot.schedules[0].state, next_schedule.clone());
            assert_eq!(snapshot.attempts, expected_attempts);
            assert_eq!(snapshot.applied_reviews.len(), 1);
            assert_eq!(
                snapshot.applied_reviews[0].key,
                "idempotency:idempotent-live-a"
            );
            assert_eq!(snapshot.applied_reviews[0].attempt, attempt);
            assert_eq!(snapshot.content_feedback.len(), 1);
            assert_eq!(snapshot.content_feedback[0].id, "feedback-live-a");
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

        run_postgres_prompt_edit_contract(store)?;

        Ok(())
    }

    fn run_postgres_prompt_edit_contract(
        store: &mut PostgresStudyStore,
    ) -> Result<(), PostgresStoreError> {
        let review_unit_id = ReviewUnitId::new("unit-live-prompt");
        let source = source_document("source-live-prompt");
        let reference = reference_span("reference-live-prompt", &source.id);
        let mut draft = accepted_draft(
            "draft-live-prompt",
            &review_unit_id,
            &[&source.id],
            &[&reference.id],
            Some("run-live-prompt"),
        );
        draft.prompt = Prompt::Mcq {
            review_unit_id: review_unit_id.clone(),
            prompt: "Original prompt".to_owned(),
            choices: vec!["Original answer".to_owned(), "Distractor".to_owned()],
            correct_choice: "Original answer".to_owned(),
        };
        let run = generation_run("run-live-prompt", &[&source.id], &[&draft.id]);
        let mut account = store.for_account(AccountScope::new("acct_live_prompt")?);
        account.ensure_account(NOW)?;
        account.save_source_document(&source)?;
        account.save_reference_span(&reference)?;
        account.save_generation_run(&run)?;
        account.save_generated_prompt_draft(&draft)?;
        account.save_review_unit(&review_unit(&draft))?;

        let updated = account.update_review_unit_prompt_text(
            &review_unit_id,
            "Edited prompt",
            "Edited answer",
        )?;
        assert_eq!(updated.prompt, {
            let mut expected = draft.prompt.clone();
            replace_prompt_text(&mut expected, "Edited prompt");
            replace_prompt_answer(&mut expected, "Edited answer").expect("valid edited answer");
            expected
        });
        match &updated.prompt {
            Prompt::Mcq {
                choices,
                correct_choice,
                ..
            } => {
                assert_eq!(correct_choice, "Edited answer");
                assert!(choices.iter().any(|choice| choice == "Edited answer"));
            }
            prompt => panic!("expected edited MCQ prompt, got {prompt:?}"),
        }

        run_postgres_boolean_prompt_edit_contract(&mut account)?;
        let snapshot = account.snapshot()?;
        let edited_draft = snapshot
            .generated_prompt_drafts
            .iter()
            .find(|draft| draft.id == "draft-live-prompt")
            .expect("edited prompt draft");
        let edited_prompt = match &edited_draft.prompt {
            Prompt::Mcq { prompt, .. } | Prompt::Boolean { prompt, .. } => prompt.as_str(),
            Prompt::Exact(prompt) => prompt.prompt.as_str(),
        };
        assert_eq!(edited_prompt, "Edited prompt");
        assert!(edited_draft
            .critique_notes
            .iter()
            .any(|note| note == "Learner edited approved wording."));
        account.archive_review_unit(&review_unit_id, NOW + 1_000)?;
        assert!(matches!(
            account.update_review_unit_prompt_text(
                &review_unit_id,
                "Rejected prompt",
                "Rejected answer"
            ),
            Err(PostgresStoreError::ReviewUnitArchived(id)) if id == review_unit_id
        ));
        assert!(matches!(
            account.snooze_review_unit_until(&review_unit_id, NOW + 2_000),
            Err(PostgresStoreError::ReviewUnitArchived(id)) if id == review_unit_id
        ));
        Ok(())
    }

    fn record_live_content_feedback(
        account: &mut super::AccountStudyStore<'_>,
        review_unit_id: &ReviewUnitId,
    ) -> Result<(), PostgresStoreError> {
        record_content_feedback(
            account,
            RecordContentFeedbackCommand {
                feedback_id: "feedback-live-a".to_owned(),
                review_unit_id: review_unit_id.clone(),
                verdict: ContentFeedbackVerdict::Dropped,
                rationale: Some("The live fixture is too easy.".to_owned()),
                account_id: "acct_live_a".to_owned(),
                occurred_at: NOW,
                supersedes_id: None,
            },
        )
        .map_err(|error| PostgresStoreError::StudySession(error.to_string()))?;
        assert!(account
            .export_content_feedback_json()?
            .contains("gen_ai.prompt.version"));
        Ok(())
    }

    fn run_postgres_boolean_prompt_edit_contract(
        account: &mut AccountStudyStore<'_>,
    ) -> Result<(), PostgresStoreError> {
        let review_unit_id = ReviewUnitId::new("unit-live-boolean-prompt");
        let source = source_document("source-live-boolean-prompt");
        let reference = reference_span("reference-live-boolean-prompt", &source.id);
        let mut draft = accepted_draft(
            "draft-live-boolean-prompt",
            &review_unit_id,
            &[&source.id],
            &[&reference.id],
            Some("run-live-boolean-prompt"),
        );
        draft.prompt = Prompt::Boolean {
            review_unit_id: review_unit_id.clone(),
            prompt: "Original Boolean prompt".to_owned(),
            correct_answer: true,
        };
        let run = generation_run("run-live-boolean-prompt", &[&source.id], &[&draft.id]);
        account.save_source_document(&source)?;
        account.save_reference_span(&reference)?;
        account.save_generation_run(&run)?;
        account.save_generated_prompt_draft(&draft)?;
        account.save_review_unit(&review_unit(&draft))?;
        let before_invalid = account.snapshot()?;
        assert!(matches!(
            account.update_review_unit_prompt_text(
                &review_unit_id,
                "Changed Boolean prompt",
                "maybe"
            ),
            Err(PostgresStoreError::InvalidBooleanAnswer)
        ));
        assert_eq!(account.snapshot()?, before_invalid);
        let updated = account.update_review_unit_prompt_text(
            &review_unit_id,
            "Changed Boolean prompt",
            "  FALSE ",
        )?;
        assert!(matches!(
            updated.prompt,
            Prompt::Boolean {
                correct_answer: false,
                ..
            }
        ));
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

            let schedule_before_edit = approved
                .current
                .as_ref()
                .and_then(|current| current.review_state.clone());
            let edited =
                study.edit_current_prompt("Edited NATO composition prompt", "EDITED CAT")?;
            assert_eq!(edited.status, BetaStudyStatus::Answering);
            assert_eq!(
                edited
                    .current
                    .as_ref()
                    .map(|current| current.prompt.as_str()),
                Some("Edited NATO composition prompt")
            );
            assert_eq!(
                edited
                    .current
                    .as_ref()
                    .map(|current| current.revision_expected_answer.as_str()),
                Some("EDITED CAT")
            );
            assert_eq!(
                edited
                    .current
                    .as_ref()
                    .and_then(|current| current.review_state.clone()),
                schedule_before_edit
            );

            let revealed = study.reveal()?;
            assert_eq!(
                revealed
                    .current
                    .as_ref()
                    .and_then(|current| current.expected_answer.as_deref()),
                Some("EDITED CAT")
            );

            let reviewed = study.submit_answer("EDITED CAT", 4_200)?;
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
            let edited_draft = snapshot
                .generated_prompt_drafts
                .iter()
                .find(|draft| {
                    draft
                        .review_unit_id
                        .as_str()
                        .ends_with("nato-cat-composition")
                })
                .expect("edited draft");
            let edited_prompt = serde_json::to_string(&edited_draft.prompt)?;
            assert!(edited_prompt.contains("Edited NATO composition prompt"));
            assert!(edited_prompt.contains("EDITED CAT"));
            assert!(edited_draft
                .critique_notes
                .iter()
                .any(|note| note == "Learner edited approved wording."));
            assert_eq!(
                edited_draft.validation.status,
                GeneratedPromptValidationStatus::Accepted
            );
            assert_eq!(snapshot.review_units.len(), 1);
            assert_eq!(snapshot.attempts.len(), 1);
            assert_eq!(snapshot.applied_reviews.len(), 1);
        }

        Ok(())
    }

    fn run_postgres_concept_snooze_contract(
        store: &mut PostgresStudyStore,
    ) -> Result<(), PostgresStoreError> {
        let mut account = store.for_account(AccountScope::new("acct_live_concept_snooze")?);
        account.ensure_account(NOW)?;

        for (review_unit_id, concept_key) in [
            ("concept-snooze-live-a", "  shared-live-concept  "),
            ("concept-snooze-live-b", "  shared-live-concept  "),
            ("concept-snooze-live-other", "other-live-concept"),
        ] {
            let review_unit_id = ReviewUnitId::new(review_unit_id);
            let mut unit = review_unit_for_concept(&review_unit_id, concept_key);
            unit.queue.due = NOW - 60_000;
            account.save_review_unit(&unit)?;
            account.set_schedule_state(
                &review_unit_id,
                Some(&schedule_state(2, ScheduleStatus::Review, NOW - 86_400_000)),
                NOW,
            )?;
        }

        let blank_id = ReviewUnitId::new("concept-snooze-live-blank");
        let mut blank = review_unit_for_concept(&blank_id, "");
        blank.queue.due = NOW + 60_000;
        account.save_review_unit(&blank)?;
        let mut blank_schedule = schedule_state(2, ScheduleStatus::Review, NOW);
        blank_schedule.due = NOW + 60_000;
        account.set_schedule_state(&blank_id, Some(&blank_schedule), NOW)?;
        let before_blank_rejection = account.snapshot()?;
        assert!(matches!(
            account.snooze_current_review_unit_concept_until(
                blank_id.as_str(),
                NOW,
                NOW + 86_400_000,
            ),
            Err(PostgresStoreError::UnknownReviewUnit(id)) if id == blank_id
        ));
        assert_eq!(account.snapshot()?, before_blank_rejection);

        let before = account.snapshot()?;
        let snoozed = account
            .snooze_review_units_for_concept_until("  shared-live-concept  ", NOW + 86_400_000)?;
        assert_eq!(snoozed.len(), 2);
        assert!(snoozed
            .iter()
            .all(|unit| unit.snoozed_until == Some(NOW + 86_400_000)));

        let after = account.snapshot()?;
        assert_eq!(after.attempts, before.attempts);
        assert_eq!(after.schedules, before.schedules);
        assert_eq!(
            after
                .review_units
                .iter()
                .find(|unit| unit.review_unit_id == ReviewUnitId::new("concept-snooze-live-other"))
                .and_then(|unit| unit.snoozed_until),
            None
        );
        assert_eq!(
            after
                .review_units
                .iter()
                .filter(|unit| unit.queue.concept_key.as_deref() == Some("  shared-live-concept  "))
                .filter_map(|unit| unit.snoozed_until)
                .collect::<Vec<_>>(),
            vec![NOW + 86_400_000; 2]
        );

        Ok(())
    }

    fn source_document(id: &str) -> SourceDocument {
        SourceDocument {
            id: id.to_owned(),
            kind: SourceDocumentKind::Text,
            title: "Live Postgres source".to_owned(),
            project_key: None,
            body: Some("Pater noster means Our Father.".to_owned()),
            uri: None,
            permission: SourcePermission::ModelEligible,
            freshness: Some(NOW),
            ttl_expires_at: None,
            created_at: NOW,
            archived_at: None,
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
            project_key: None,
            ttl_expires_at: None,
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
            parent_review_unit_id: None,
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
            concept_reference_note_key: None,
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
            concept_reference_note_key: None,
            generated_prompt_draft_id: Some(draft.id.clone()),
            archived_at: None,
            snoozed_until: None,
            created_at: NOW,
        }
    }

    fn review_unit_for_concept(
        review_unit_id: &ReviewUnitId,
        concept_key: &str,
    ) -> BetaReviewUnitRecord {
        BetaReviewUnitRecord {
            review_unit_id: review_unit_id.clone(),
            prompt_id: format!("prompt-{review_unit_id}"),
            prompt: prompt(review_unit_id),
            queue: PersistedQueueCandidate {
                review_unit_id: review_unit_id.clone(),
                due: NOW - 60_000,
                lifecycle: ReviewUnitLifecycle::active(),
                progression: None,
                concept_key: Some(concept_key.to_owned()),
                source_key: Some("live-concept-source".to_owned()),
                domain_key: Some("live".to_owned()),
            },
            reference_span_ids: Vec::new(),
            concept_reference_note_key: None,
            generated_prompt_draft_id: None,
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
            lifecycle: ReviewUnitLifecycle::active(),
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
