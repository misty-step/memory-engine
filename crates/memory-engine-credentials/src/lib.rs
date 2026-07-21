//! Shared credential resolution and on-disk storage for memory-engine's
//! Bearer-token clients: `memory-engine-review` (CLI) and `memory-engine-mcp`
//! (stdio MCP server). One default file
//! (`$MEMORY_ENGINE_HOME/credentials.json`, `~/.memory-engine/credentials.json`
//! when unset) so `memory-engine-review login` and a freshly started
//! `memory-engine-mcp` agree on the same account without an operator manually
//! copying a credentials file between two clients' separate state
//! directories — the drift `memory-engine-mcp-production-parity` closed
//! (each client previously wrote its own subdirectory, so logging in with one
//! did not make the other work).

use std::{
    env, fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

/// Branded production origin. `scry.study` is a custom domain in front of
/// the same `DigitalOcean` App Platform deployment `docs/runbook.md` documents
/// and health-checks directly.
pub const DEFAULT_BASE_URL: &str = "https://scry.study";

/// The `DigitalOcean` App Platform origin `scry.study` fronts. Still live and
/// still the identity `docs/runbook.md`'s deploy smoke checks against, but no
/// longer the default a client advertises. Pass it explicitly
/// (`--base-url` / `MEMORY_ENGINE_MCP_BASE_URL`) only as an operator-origin
/// fallback, e.g. while DNS for the branded domain is degraded.
pub const OPERATOR_ORIGIN_FALLBACK_BASE_URL: &str =
    "https://memory-engine-api-i2xcr.ondigitalocean.app";

/// The shape persisted to `default_credentials_path()`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredCredentials {
    pub base_url: String,
    pub account_id: String,
    pub session_token: String,
}

/// A resolved session: which origin to call, and the Bearer credential to
/// call it with.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Session {
    pub base_url: String,
    pub account_id: String,
    pub session_token: String,
}

/// `$MEMORY_ENGINE_HOME`, or `$HOME/.memory-engine` when unset.
#[must_use]
pub fn memory_engine_home() -> PathBuf {
    non_empty_env("MEMORY_ENGINE_HOME").map_or_else(
        || {
            let home = non_empty_env("HOME").unwrap_or_else(|| ".".to_owned());
            PathBuf::from(home).join(".memory-engine")
        },
        PathBuf::from,
    )
}

/// The one credentials file every Bearer-token client reads and writes.
/// `memory-engine-review login` and `memory-engine-mcp`'s email bootstrap
/// both resolve here, so authenticating once with either client is enough
/// for both.
#[must_use]
pub fn default_credentials_path() -> PathBuf {
    memory_engine_home().join("credentials.json")
}

#[must_use]
pub fn non_empty_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// Resolve a session from `MEMORY_ENGINE_ACCOUNT_ID` / `MEMORY_ENGINE_SESSION_TOKEN`.
/// Both must be set (and non-blank) for either to count — a lone env var is
/// treated as absent, never a partial session.
#[must_use]
pub fn env_session(base_url_override: Option<String>, default_base_url: &str) -> Option<Session> {
    let account_id = non_empty_env("MEMORY_ENGINE_ACCOUNT_ID")?;
    let session_token = non_empty_env("MEMORY_ENGINE_SESSION_TOKEN")?;
    Some(Session {
        base_url: base_url_override.unwrap_or_else(|| default_base_url.to_owned()),
        account_id,
        session_token,
    })
}

/// # Errors
///
/// Returns an error when `path` exists but is not valid JSON, or cannot be
/// read for a reason other than not existing.
pub fn read_credentials(path: &Path) -> Result<Option<StoredCredentials>, String> {
    match fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents)
            .map(Some)
            .map_err(|error| format!("malformed credentials at {}: {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("could not read {}: {error}", path.display())),
    }
}

/// # Errors
///
/// Returns an error when the parent directory cannot be created or the file
/// cannot be opened/written at mode `0600`.
pub fn write_credentials(path: &Path, credentials: &StoredCredentials) -> Result<(), String> {
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

// Creates (or truncates) the file with mode 0600 set at open time, on Unix:
// no window where the session token is readable under the ambient umask
// before a follow-up chmod narrows it.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "memory-engine-credentials-test-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |duration| duration.as_nanos())
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn round_trip_credentials_file_restricts_permissions_on_unix() {
        let dir = tempdir("round-trip");
        let path = dir.join("credentials.json");
        let credentials = StoredCredentials {
            base_url: DEFAULT_BASE_URL.to_owned(),
            account_id: "acct_demo".to_owned(),
            session_token: "secret".to_owned(),
        };
        write_credentials(&path, &credentials).expect("write credentials");
        let read_back = read_credentials(&path)
            .expect("read credentials")
            .expect("present");
        assert_eq!(read_back, credentials);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn read_credentials_returns_none_when_file_is_missing() {
        let dir = tempdir("missing");
        let path = dir.join("nope.json");
        assert_eq!(read_credentials(&path).expect("read"), None);
    }

    #[test]
    fn read_credentials_rejects_malformed_json() {
        let dir = tempdir("malformed");
        let path = dir.join("credentials.json");
        fs::write(&path, "not json").expect("write malformed file");
        let error = read_credentials(&path).expect_err("malformed credentials must error");
        assert!(error.contains("malformed credentials"));
    }

    #[test]
    fn env_session_requires_both_variables_together() {
        // SAFETY: this test does not race other tests over these two names
        // within this crate's own test binary.
        env::remove_var("MEMORY_ENGINE_ACCOUNT_ID");
        env::remove_var("MEMORY_ENGINE_SESSION_TOKEN");
        assert_eq!(env_session(None, DEFAULT_BASE_URL), None);

        env::set_var("MEMORY_ENGINE_ACCOUNT_ID", "acct_env");
        assert_eq!(
            env_session(None, DEFAULT_BASE_URL),
            None,
            "a lone account id must not resolve a session"
        );

        env::set_var("MEMORY_ENGINE_SESSION_TOKEN", "env-token");
        let session = env_session(None, DEFAULT_BASE_URL).expect("both vars set");
        assert_eq!(session.account_id, "acct_env");
        assert_eq!(session.session_token, "env-token");
        assert_eq!(session.base_url, DEFAULT_BASE_URL);

        env::remove_var("MEMORY_ENGINE_ACCOUNT_ID");
        env::remove_var("MEMORY_ENGINE_SESSION_TOKEN");
    }

    #[test]
    fn default_credentials_path_is_one_shared_file() {
        env::set_var(
            "MEMORY_ENGINE_HOME",
            "/tmp/memory-engine-credentials-shared-home",
        );
        assert_eq!(
            default_credentials_path(),
            Path::new("/tmp/memory-engine-credentials-shared-home/credentials.json")
        );
        env::remove_var("MEMORY_ENGINE_HOME");
    }
}
