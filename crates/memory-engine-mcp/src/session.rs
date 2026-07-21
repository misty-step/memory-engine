//! Credential resolution for the stdio server.
//!
//! Uses `memory-engine-credentials` (same crate `memory-engine-review` uses)
//! for the env var names, on-disk format, and default file path, so
//! `memory-engine-review login` and this server agree on one account without
//! manual copying. There is no interactive `login` subcommand: stdin is the
//! JSON-RPC channel, not a terminal. A brand-new local server bootstraps its
//! own account non-interactively from `MEMORY_ENGINE_MCP_EMAIL` instead.
//! There is no silent in-memory fallback — if none of the three paths below
//! resolve, the server fails loudly at startup rather than guessing.

use memory_engine_credentials::{
    env_session, read_credentials, write_credentials, StoredCredentials, DEFAULT_BASE_URL,
};

use crate::client;

pub use memory_engine_credentials::{Session, OPERATOR_ORIGIN_FALLBACK_BASE_URL};

/// Resolve credentials in order: `MEMORY_ENGINE_ACCOUNT_ID` /
/// `MEMORY_ENGINE_SESSION_TOKEN` env vars, then the shared credentials file
/// (migrating a legacy per-client file into it first if the shared file is
/// missing — see
/// `memory_engine_credentials::resolve_default_credentials_path`), then a
/// fresh account created from `MEMORY_ENGINE_MCP_EMAIL` (persisted to the
/// shared file for reuse across restarts). The shared path is resolved
/// lazily, after the env-var check: a caller relying on
/// `MEMORY_ENGINE_ACCOUNT_ID`/`MEMORY_ENGINE_SESSION_TOKEN` never touches
/// (or migrates) any file on disk.
///
/// # Errors
///
/// Returns an error describing every path that was tried when none
/// resolves, when a legacy migration conflict is found, or when account
/// creation fails.
pub fn resolve() -> Result<Session, String> {
    let base_url_override = memory_engine_credentials::non_empty_env("MEMORY_ENGINE_MCP_BASE_URL");

    if let Some(session) = env_session(base_url_override.clone(), DEFAULT_BASE_URL) {
        return Ok(session);
    }

    let credentials_path = memory_engine_credentials::resolve_default_credentials_path()?;

    if let Some(stored) = read_credentials(&credentials_path)? {
        return Ok(Session {
            base_url: base_url_override.unwrap_or(stored.base_url),
            account_id: stored.account_id,
            session_token: stored.session_token,
        });
    }

    if let Some(email) = memory_engine_credentials::non_empty_env("MEMORY_ENGINE_MCP_EMAIL") {
        let base_url = base_url_override.unwrap_or_else(|| DEFAULT_BASE_URL.to_owned());
        let created = client::create_account(&base_url, &email)?;
        write_credentials(
            &credentials_path,
            &StoredCredentials {
                base_url: base_url.clone(),
                account_id: created.account_id.clone(),
                session_token: created.session_token.clone(),
            },
        )?;
        return Ok(Session {
            base_url,
            account_id: created.account_id,
            session_token: created.session_token,
        });
    }

    Err(format!(
        "no memory-engine credentials found. Set MEMORY_ENGINE_ACCOUNT_ID and \
         MEMORY_ENGINE_SESSION_TOKEN (see docs/runbook.md), or a credentials file at {} \
         (written by this server or `memory-engine-review login` — one shared file, no manual \
         copying between clients), or MEMORY_ENGINE_MCP_EMAIL to bootstrap a fresh account. \
         There is no in-memory fallback: a stdio MCP server that silently created and discarded \
         an account would leave an agent believing its study state persisted when it did not.",
        credentials_path.display()
    ))
}

pub use memory_engine_credentials::default_credentials_path;

#[cfg(test)]
mod tests {
    use std::{env, fs, path::PathBuf, sync::Mutex};

    use super::*;

    // `resolve` reads process-global env vars; the default test harness runs
    // tests in parallel threads, so any test that sets/removes
    // MEMORY_ENGINE_ACCOUNT_ID/SESSION_TOKEN/MCP_EMAIL/HOME must serialize
    // against every other test that does, or one test's `remove_var` races
    // another's `set_var`. This lock is that serialization point.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn resolve_prefers_env_vars_over_credentials_file() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempdir("session-env-precedence");
        env::set_var("MEMORY_ENGINE_HOME", &home);
        write_credentials(
            &home.join("credentials.json"),
            &StoredCredentials {
                base_url: "https://file.example.com".to_owned(),
                account_id: "acct_file".to_owned(),
                session_token: "file-token".to_owned(),
            },
        )
        .expect("write credentials");

        env::set_var("MEMORY_ENGINE_ACCOUNT_ID", "acct_env");
        env::set_var("MEMORY_ENGINE_SESSION_TOKEN", "env-token");
        let session = resolve().expect("session");
        env::remove_var("MEMORY_ENGINE_ACCOUNT_ID");
        env::remove_var("MEMORY_ENGINE_SESSION_TOKEN");
        env::remove_var("MEMORY_ENGINE_HOME");

        assert_eq!(session.account_id, "acct_env");
        assert_eq!(session.session_token, "env-token");
    }

    #[test]
    fn resolve_fails_loudly_with_no_credentials_and_no_bootstrap_email() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempdir("session-no-fallback");
        env::set_var("MEMORY_ENGINE_HOME", &home);

        env::remove_var("MEMORY_ENGINE_ACCOUNT_ID");
        env::remove_var("MEMORY_ENGINE_SESSION_TOKEN");
        env::remove_var("MEMORY_ENGINE_MCP_EMAIL");

        let error = resolve().expect_err("must fail without any credential path");
        env::remove_var("MEMORY_ENGINE_HOME");
        assert!(error.contains("no memory-engine credentials found"));
    }

    #[test]
    fn resolve_migrates_a_legacy_mcp_credentials_file_before_falling_back_to_email_bootstrap() {
        // The sharper regression this closes: an MCP host upgraded with
        // MEMORY_ENGINE_MCP_EMAIL still set (the documented setup) and a
        // pre-existing legacy `mcp/credentials.json` must keep resolving to
        // that same account, not silently bootstrap a brand-new one and
        // make the agent's prior study state appear to vanish. A garbage
        // base url guards against ever reaching a real network call if
        // migration regresses and bootstrap is (wrongly) attempted.
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempdir("session-migrate-before-bootstrap");
        env::set_var("MEMORY_ENGINE_HOME", &home);
        write_credentials(
            &home.join("mcp").join("credentials.json"),
            &StoredCredentials {
                base_url: "https://file.example.com".to_owned(),
                account_id: "acct_legacy_mcp".to_owned(),
                session_token: "legacy-mcp-token".to_owned(),
            },
        )
        .expect("seed legacy mcp credentials file");

        env::remove_var("MEMORY_ENGINE_ACCOUNT_ID");
        env::remove_var("MEMORY_ENGINE_SESSION_TOKEN");
        env::set_var(
            "MEMORY_ENGINE_MCP_EMAIL",
            "should-not-bootstrap@example.com",
        );
        env::set_var("MEMORY_ENGINE_MCP_BASE_URL", "http://127.0.0.1:1");

        let session = resolve().expect("session resolves from the migrated legacy file");
        env::remove_var("MEMORY_ENGINE_MCP_EMAIL");
        env::remove_var("MEMORY_ENGINE_MCP_BASE_URL");
        env::remove_var("MEMORY_ENGINE_HOME");

        assert_eq!(
            session.account_id, "acct_legacy_mcp",
            "must reuse the migrated account, never silently bootstrap a new one"
        );
        assert_eq!(session.session_token, "legacy-mcp-token");
        assert!(
            !home.join("mcp").join("credentials.json").exists(),
            "the legacy file must be migrated (removed), not left as a permanent dual-read shim"
        );
    }

    fn tempdir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "memory-engine-mcp-test-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |duration| duration.as_nanos())
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }
}
