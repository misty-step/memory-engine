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

use std::path::Path;

use memory_engine_credentials::{
    env_session, read_credentials, write_credentials, StoredCredentials, DEFAULT_BASE_URL,
};

use crate::client;

pub use memory_engine_credentials::{Session, OPERATOR_ORIGIN_FALLBACK_BASE_URL};

/// Resolve credentials in order: `MEMORY_ENGINE_ACCOUNT_ID` /
/// `MEMORY_ENGINE_SESSION_TOKEN` env vars, then `credentials_path`, then a
/// fresh account created from `MEMORY_ENGINE_MCP_EMAIL` (persisted to
/// `credentials_path` for reuse across restarts).
///
/// # Errors
///
/// Returns an error describing every path that was tried when none resolves,
/// or when account creation fails.
pub fn resolve(credentials_path: &Path) -> Result<Session, String> {
    let base_url_override = memory_engine_credentials::non_empty_env("MEMORY_ENGINE_MCP_BASE_URL");

    if let Some(session) = env_session(base_url_override.clone(), DEFAULT_BASE_URL) {
        return Ok(session);
    }

    if let Some(stored) = read_credentials(credentials_path)? {
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
            credentials_path,
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
    // MEMORY_ENGINE_ACCOUNT_ID/SESSION_TOKEN/MCP_EMAIL must serialize against
    // every other test that does, or one test's `remove_var` races another's
    // `set_var`. This lock is that serialization point.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn resolve_prefers_env_vars_over_credentials_file() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempdir("session-env-precedence");
        let credentials_path = dir.join("credentials.json");
        write_credentials(
            &credentials_path,
            &StoredCredentials {
                base_url: "https://file.example.com".to_owned(),
                account_id: "acct_file".to_owned(),
                session_token: "file-token".to_owned(),
            },
        )
        .expect("write credentials");

        env::set_var("MEMORY_ENGINE_ACCOUNT_ID", "acct_env");
        env::set_var("MEMORY_ENGINE_SESSION_TOKEN", "env-token");
        let session = resolve(&credentials_path).expect("session");
        env::remove_var("MEMORY_ENGINE_ACCOUNT_ID");
        env::remove_var("MEMORY_ENGINE_SESSION_TOKEN");

        assert_eq!(session.account_id, "acct_env");
        assert_eq!(session.session_token, "env-token");
    }

    #[test]
    fn resolve_fails_loudly_with_no_credentials_and_no_bootstrap_email() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempdir("session-no-fallback");
        let credentials_path = dir.join("missing.json");

        env::remove_var("MEMORY_ENGINE_ACCOUNT_ID");
        env::remove_var("MEMORY_ENGINE_SESSION_TOKEN");
        env::remove_var("MEMORY_ENGINE_MCP_EMAIL");

        let error = resolve(&credentials_path).expect_err("must fail without any credential path");
        assert!(error.contains("no memory-engine credentials found"));
    }

    #[test]
    fn resolved_credentials_path_matches_the_shared_default() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        env::set_var(
            "MEMORY_ENGINE_HOME",
            "/tmp/memory-engine-mcp-session-shared-home",
        );
        assert_eq!(
            default_credentials_path(),
            PathBuf::from("/tmp/memory-engine-mcp-session-shared-home/credentials.json")
        );
        env::remove_var("MEMORY_ENGINE_HOME");
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
