#![cfg_attr(not(test), deny(clippy::expect_used, clippy::unwrap_used))]

//! Production HTTP API boundary for the mobile study app.
//!
//! This crate owns the HTTP surface. Account/session state, persistence
//! adapters, background jobs, and HTML rendering live in smaller boundary
//! crates so `memory-engine-core` stays pure learning semantics.

mod routes;

#[cfg(test)]
mod tests;

pub use memory_engine_api_state::{
    init_error_reporting, report_health_check_in, start_health_reporting_loop, AccountRegistry,
    ApiState, AuthConfig,
};

#[cfg(test)]
pub use memory_engine_api_state::{
    app_session_max_age_ms, AccountCreated, ApiError, ApiFailure, AppAccount, AuthLinkDelivery,
    CreateAccountRequest, CreateProjectDeckRequest, CreateSourceRequest, GenerationJob,
    HealthResponse, InvalidateProjectDeckRequest, JobBroadcast, JobQueue, JobStatus,
    ProjectDeckRecord, ReadinessResponse, ReturnNotificationSchedulerConfig, SourceList,
    SourceRecord, StudyStorage, StudyViewResponse, SubmitReviewRequest,
    APP_ACCOUNT_RATE_LIMIT_MAX_ATTEMPTS, APP_SESSION_COOKIE_NAME, AUTH_CHALLENGE_TTL_MS,
    RETURN_NOTIFICATION_UNSUBSCRIBE_TTL_MS,
};

pub use routes::router;
