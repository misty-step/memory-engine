//! Credential resolution for the stdio server.
//!
//! Mirrors `memory-engine-review`'s credential model exactly (same env var
//! names `docs/runbook.md` already documents), but with no interactive
//! `login` subcommand: stdin is the JSON-RPC channel, not a terminal. A
//! brand-new local server bootstraps its own account non-interactively from
//! `MEMORY_ENGINE_MCP_EMAIL` instead. There is no silent in-memory fallback —
//! if none of the three paths below resolve, the server fails loudly at
//! startup rather than guessing.

use std::{
    env, fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::client;

pub const DEFAULT_BASE_URL: &str = "https://memory-engine-api-i2xcr.ondigitalocean.app";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredCredentials {
    base_url: String,
    account_id: String,
    session_token: String,
}

#[derive(Debug)]
pub struct Session {
    pub base_url: String,
    pub account_id: String,
    pub session_token: String,
}

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
    let base_url =
        non_empty_env("MEMORY_ENGINE_MCP_BASE_URL").unwrap_or_else(|| DEFAULT_BASE_URL.to_owned());

    if let (Some(account_id), Some(session_token)) = (
        non_empty_env("MEMORY_ENGINE_ACCOUNT_ID"),
        non_empty_env("MEMORY_ENGINE_SESSION_TOKEN"),
    ) {
        return Ok(Session {
            base_url,
            account_id,
            session_token,
        });
    }

    if let Some(stored) = read_credentials(credentials_path)? {
        return Ok(Session {
            base_url: stored.base_url,
            account_id: stored.account_id,
            session_token: stored.session_token,
        });
    }

    if let Some(email) = non_empty_env("MEMORY_ENGINE_MCP_EMAIL") {
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
         (written by this server or `memory-engine-review login`), or MEMORY_ENGINE_MCP_EMAIL \
         to bootstrap a fresh account. There is no in-memory fallback: a stdio MCP server \
         that silently created and discarded an account would leave an agent believing its \
         study state persisted when it did not.",
        credentials_path.display()
    ))
}

#[must_use]
pub fn default_credentials_path() -> PathBuf {
    memory_engine_home().join("mcp").join("credentials.json")
}

fn memory_engine_home() -> PathBuf {
    non_empty_env("MEMORY_ENGINE_HOME").map_or_else(
        || {
            let home = non_empty_env("HOME").unwrap_or_else(|| ".".to_owned());
            PathBuf::from(home).join(".memory-engine")
        },
        PathBuf::from,
    )
}

fn read_credentials(path: &Path) -> Result<Option<StoredCredentials>, String> {
    match fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents)
            .map(Some)
            .map_err(|error| format!("malformed credentials at {}: {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("could not read {}: {error}", path.display())),
    }
}

fn write_credentials(path: &Path, credentials: &StoredCredentials) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let serialized =
        serde_json::to_string_pretty(credentials).map_err(|error| error.to_string())?;
    let mut file = open_restricted(path)?;
    std::io::Write::write_all(&mut file, serialized.as_bytes())
        .map_err(|error| format!("could not write {}: {error}", path.display()))
}

// Creates (or truncates) the file with mode 0600 set at open time, on Unix,
// matching `memory-engine-review`'s credential file: no window where the
// session token is readable under the ambient umask before a follow-up
// chmod narrows it.
#[cfg(unix)]
fn open_restricted(path: &Path) -> Result<fs::File, String> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| format!("could not open {}: {error}", path.display()))
}

#[cfg(not(unix))]
fn open_restricted(path: &Path) -> Result<fs::File, String> {
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .map_err(|error| format!("could not open {}: {error}", path.display()))
}

fn non_empty_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

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
