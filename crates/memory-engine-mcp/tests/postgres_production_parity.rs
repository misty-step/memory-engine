//! Production-shaped proof: runs `memory-engine-mcp`'s HTTP client against a
//! Postgres-backed `ApiState` (not the ephemeral `ApiState::default()` file
//! store every other test in this crate uses), the same backend selection
//! `memory-engine-api`'s `main.rs` makes when `MEMORY_ENGINE_POSTGRES_URL` is
//! set — i.e. every real production deployment.
//!
//! Two things this proves that a file-store fixture cannot:
//!
//! 1. **Reproduces the reported production failure directly**: the legacy
//!    synchronous `/generate` route returns HTTP 409 with the exact message
//!    `registry.rs::generate_source` emits once Postgres is configured
//!    (`Direct synchronous generation is disabled in production...`) — the
//!    bug this ticket exists to route around, not paper over.
//! 2. **Proves the fix against that same backend**: `MemoryEngineClient`'s
//!    queued composition (`create_deck` → enqueue → poll `generation-jobs`)
//!    reaches `succeeded` leaving its draft pending a learner decision, and a
//!    keep then schedules a due card on the Postgres store,
//!    the exact path `docs/qa/103-machine-generation-receipt-2026-07-17.md`
//!    proved live against `scry.study`.
//!
//! Skipped when `MEMORY_ENGINE_POSTGRES_TEST_URL` is unset (the same
//! convention every other Postgres-gated test in this workspace uses); point
//! it at a scratch database, e.g. a local `initdb`/`pg_ctl` instance — this
//! test does not require the Dagger `bun run ci:full` lane.

use memory_engine_mcp::client::MemoryEngineClient;
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_generation_succeeds_on_postgres_where_the_legacy_route_is_refused() {
    let Some(database_url) = std::env::var("MEMORY_ENGINE_POSTGRES_TEST_URL").ok() else {
        eprintln!(
            "skipping Postgres production-parity test; MEMORY_ENGINE_POSTGRES_TEST_URL is unset"
        );
        return;
    };

    let email = format!("memory-engine-mcp-pg-test-{}@example.com", unique_suffix());
    let state = memory_engine_api::ApiState::new(
        memory_engine_api::AccountRegistry::with_postgres_url(database_url).with_auth_config(
            memory_engine_api::AuthConfig::allow_emails([email.clone()])
                .with_anonymous_account_creation(true),
        ),
    );
    state.start_worker();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local API listener");
    let address = listener.local_addr().expect("local address");
    let server = tokio::spawn(async move {
        axum::serve(listener, memory_engine_api::router(state))
            .await
            .expect("serve local API");
    });

    let base_url = format!("http://{address}");
    let created: serde_json::Value = ureq::post(format!("{base_url}/v1/accounts"))
        .send_json(json!({ "email": email }))
        .expect("create account")
        .body_mut()
        .read_json()
        .expect("account json");
    let account_id = created["accountId"].as_str().expect("accountId").to_owned();
    let session_token = created["sessionToken"]
        .as_str()
        .expect("sessionToken")
        .to_owned();
    let authorization = format!("Bearer {session_token}");

    // Seed a saved source directly (the same v1 route the MCP client's own
    // `create_deck` uses internally via `project-decks`, called here without
    // that composition so the reproduction below targets exactly one route).
    let source: serde_json::Value = ureq::post(format!(
        "{base_url}/v1/accounts/{account_id}/sources"
    ))
    .header("Authorization", &authorization)
    .send_json(json!({
        "title": "postgres parity fixture",
        "body": "Concept: NATO letter A\nActivity: quiz\nStage: recognition-3\nQuestion: What is the NATO phonetic alphabet word for A?\nAnswer: ALFA\nDistractors: BRAVO, CHARLIE\nReference: The NATO phonetic alphabet word for A is ALFA.",
    }))
    .expect("create source")
    .body_mut()
    .read_json()
    .expect("source json");
    let source_id = source["sourceId"].as_str().expect("sourceId").to_owned();

    // 1. Reproduce the reported production failure: the legacy synchronous
    //    route is refused outright once Postgres is configured.
    let legacy_agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let mut legacy_response = legacy_agent
        .post(format!(
            "{base_url}/v1/accounts/{account_id}/sources/{source_id}/generate"
        ))
        .header("Authorization", &authorization)
        .send_empty()
        .expect("legacy generate request completes");
    assert_eq!(
        legacy_response.status().as_u16(),
        409,
        "the legacy synchronous /generate route must be refused on Postgres"
    );
    let legacy_body: serde_json::Value = legacy_response
        .body_mut()
        .read_json()
        .expect("legacy generate error body");
    assert_eq!(
        legacy_body["error"],
        "Direct synchronous generation is disabled in production. Use the queued generation workflow.",
        "the 409 must carry the declared, agent-actionable reason, not a bare status"
    );

    // 2. Prove the fix: the same account, same backend, using the queued
    //    generation-jobs route the MCP client now composes exclusively.
    let client = MemoryEngineClient::new(base_url, account_id, session_token);
    let enqueued = client
        .enqueue_generation_job(&source_id)
        .expect("enqueue generation job on postgres backend");
    assert_eq!(enqueued.job.status, "queued");
    assert!(!enqueued.coalesced);

    let job = poll_to_terminal(&client, enqueued.job).await;
    assert_eq!(job.status, "succeeded", "job must succeed: {job:?}");
    assert_eq!(
        job.card_count, 0,
        "a succeeded job schedules nothing on its own: its draft is pending a \
         learner decision, so card_count stays 0 until the draft is kept: {job:?}"
    );

    assert_pending_until_kept(&client);

    server.abort();
}

/// Poll to a bounded terminal state directly, rather than through `create_deck`,
/// which would also create a second source: this keeps the reproduction scoped
/// to exactly the enqueue/poll/decide path.
async fn poll_to_terminal(
    client: &MemoryEngineClient,
    job: memory_engine_mcp::client::GenerationJob,
) -> memory_engine_mcp::client::GenerationJob {
    let mut job = job;
    let mut polls = 0;
    while !job.is_terminal() {
        polls += 1;
        assert!(
            polls < 60,
            "generation job did not terminate within 30s: {job:?}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        job = client
            .generation_job(&job.id)
            .expect("poll generation job on postgres backend");
    }
    job
}

/// The PR79 learner-decision gate, asserted against whatever backend `client`
/// is pointed at: a generated draft exists but is pending, nothing is scheduled
/// until it is explicitly kept, and keeping it schedules exactly one card.
fn assert_pending_until_kept(client: &MemoryEngineClient) {
    let pending = client
        .pending_drafts()
        .expect("pending drafts on postgres backend");
    assert_eq!(
        pending.len(),
        1,
        "the succeeded job must leave exactly one pending draft: {pending:?}"
    );
    assert_eq!(
        client
            .next_review()
            .expect("study view before the keep decision")
            .due_count,
        0,
        "nothing may be scheduled before the learner keeps the draft"
    );

    client
        .keep_draft(&pending[0].id)
        .expect("keep the pending draft on postgres backend");

    assert_eq!(
        client
            .next_review()
            .expect("study view after the keep decision")
            .due_count,
        1,
        "keeping the draft must schedule the generated card on postgres too"
    );
}

fn unique_suffix() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-{millis}-{counter}", std::process::id())
}
