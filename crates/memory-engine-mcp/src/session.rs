//! Credential resolution for the stdio server.
//!
//! Uses `memory-engine-credentials` (same crate `memory-engine-review` uses)
//! for the env var names, on-disk format, and default file path, so
//! `memory-engine-review login` and this server agree on one account without
//! manual copying. There is no interactive `login` subcommand: stdin is the
//! JSON-RPC channel, not a terminal, and there is no anonymous bootstrap —
//! credentials must be provisioned through invite or operator
//! service-session flows. There is no silent in-memory fallback either — if
//! none of the credential paths below resolve, the server fails loudly at
//! startup rather than guessing or minting an account.

use memory_engine_credentials::{env_session, read_credentials, DEFAULT_BASE_URL};

pub use memory_engine_credentials::{Session, OPERATOR_ORIGIN_FALLBACK_BASE_URL};

/// Resolve credentials in order: `MEMORY_ENGINE_ACCOUNT_ID` /
/// `MEMORY_ENGINE_SESSION_TOKEN` env vars, then the shared credentials file
/// (migrating a legacy per-client file into it first if the shared file is
/// missing — see
/// `memory_engine_credentials::resolve_default_credentials_path`). The
/// shared path is resolved lazily, after the env-var check: a caller relying
/// on `MEMORY_ENGINE_ACCOUNT_ID`/`MEMORY_ENGINE_SESSION_TOKEN` never touches
/// (or migrates) any file on disk. Credentials must be pre-provisioned
/// through an invite or operator service-session flow; there is no
/// anonymous bootstrap.
///
/// # Errors
///
/// Returns an error describing every path that was tried when none
/// resolves, or when a legacy migration conflict is found.
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

    Err(format!(
        "no pre-provisioned memory-engine credentials found. Set MEMORY_ENGINE_ACCOUNT_ID and \
         MEMORY_ENGINE_SESSION_TOKEN (see docs/runbook.md), or provide a credentials file at {} \
         (written by `memory-engine-review login` or an operator-managed provisioning step — \
         one shared file, no manual copying between clients). Anonymous account creation is \
         disabled; there is no in-memory fallback.",
        credentials_path.display()
    ))
}

pub use memory_engine_credentials::default_credentials_path;

#[cfg(test)]
mod tests {
    use std::{env, fs, path::PathBuf, sync::Mutex};

    use memory_engine_credentials::{write_credentials, StoredCredentials};

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
    fn resolve_fails_loudly_with_no_preprovisioned_credentials() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempdir("session-no-fallback");
        env::set_var("MEMORY_ENGINE_HOME", &home);

        env::remove_var("MEMORY_ENGINE_ACCOUNT_ID");
        env::remove_var("MEMORY_ENGINE_SESSION_TOKEN");

        let error = resolve().expect_err("must fail without any credential path");
        env::remove_var("MEMORY_ENGINE_HOME");
        assert!(error.contains("no pre-provisioned memory-engine credentials found"));
    }

    #[test]
    fn resolve_migrates_a_legacy_mcp_credentials_file_before_any_other_resolution() {
        // The regression this closes: an MCP host with a stale
        // MEMORY_ENGINE_MCP_EMAIL still set from before anonymous account
        // creation was removed, and a pre-existing legacy
        // `mcp/credentials.json`, must keep resolving to that migrated
        // account rather than treating the leftover env var as meaningful.
        // A garbage base url guards against ever reaching a real network
        // call if migration regresses and some future code path started
        // consulting MEMORY_ENGINE_MCP_EMAIL again.
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
