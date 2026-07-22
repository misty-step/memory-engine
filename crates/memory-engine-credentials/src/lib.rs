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
//!
//! `resolve_default_credentials_path` additionally performs a one-time,
//! clean-cutover migration the first time the shared file is missing:
//! it looks for either client's retired per-subdirectory file
//! (`<home>/review/credentials.json`, `<home>/mcp/credentials.json`),
//! moves the data into the shared file, and removes the legacy file so
//! there is no permanent dual-read fallback. An explicit credentials path
//! (a CLI flag, or any future explicit override) must go through
//! `default_credentials_path()`/`read_credentials()` directly instead, so
//! it never searches or migrates legacy files.

use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
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

/// Resolves `default_credentials_path()`, migrating a legacy per-client
/// credentials file into it first if the shared file does not exist yet.
///
/// This is a one-time, clean cutover: once the shared file exists — either
/// because a client wrote it directly, or because this function migrated a
/// legacy file into it — every later call returns immediately without
/// touching legacy files again; there is no permanent dual-read fallback.
/// A migrated legacy file is removed, so the state stays unambiguous going
/// forward. An untouched legacy file left behind by a resolved conflict (or
/// found after the shared file already existed) is simply ignored from
/// then on, never silently deleted.
///
/// Cross-process safe: `memory-engine-review` and `memory-engine-mcp` can
/// both start for the first time at once and race this function. The
/// install onto the shared path is atomic ([`install_shared_credentials`]
/// creates it via `hard_link`, which — unlike `rename` — fails instead of
/// silently replacing a file another racer just created), so exactly one
/// caller ever installs the shared file and no racer can overwrite a
/// concurrently installed *different* session. Every other racer re-reads
/// what is there: identical data is accepted as a benign double migration;
/// different data is a genuine conflict, reported without touching either
/// side. Legacy-file removal tolerates `NotFound` (a racer that already
/// won the install may have removed it first), so losing a benign race
/// never turns a successful migration into a spurious startup error.
///
/// Only default-path resolution goes through migration. An explicit
/// credentials path (a CLI flag such as `memory-engine-review login
/// --credentials-path`, or any future explicit override) must call
/// `default_credentials_path()`/`read_credentials()` directly instead of
/// this function, so it never searches or migrates legacy files.
///
/// # Errors
///
/// Returns an error naming every legacy path found, without their
/// contents, when more than one exists and they disagree on
/// account/session data — this never silently picks a winner. Also
/// returns an error when a legacy file is malformed, when a concurrently
/// installed shared file disagrees with the migrated data (naming only
/// the shared path, never token values), or when the migration write or
/// legacy-file removal fails for a reason other than the file already
/// being gone.
pub fn resolve_default_credentials_path() -> Result<PathBuf, String> {
    let shared = default_credentials_path();
    if shared.exists() {
        return Ok(shared);
    }

    let mut found: Vec<(PathBuf, StoredCredentials)> = Vec::new();
    for path in legacy_credentials_paths() {
        if let Some(credentials) = read_credentials(&path)? {
            found.push((path, credentials));
        }
    }

    let Some((_, first)) = found.first() else {
        return Ok(shared);
    };

    if found.iter().any(|(_, credentials)| credentials != first) {
        let paths = found
            .iter()
            .map(|(path, _)| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "found multiple legacy credentials files with conflicting account/session data \
             ({paths}); resolve manually (keep the correct one, delete the rest) before {} \
             can be written — migration never silently picks a winner",
            shared.display()
        ));
    }

    install_shared_credentials(&shared, first)?;
    for (path, _) in &found {
        remove_migrated_legacy_file(&shared, path)?;
    }

    Ok(shared)
}

/// The retired per-client credentials files `default_credentials_path()`
/// superseded. Consulted only by `resolve_default_credentials_path()`, and
/// only when the shared file is absent.
fn legacy_credentials_paths() -> [PathBuf; 2] {
    let home = memory_engine_home();
    [
        home.join("review").join("credentials.json"),
        home.join("mcp").join("credentials.json"),
    ]
}

/// Atomically installs `credentials` at `shared` the first time it is
/// created, so two processes racing to migrate the same (or agreeing)
/// legacy files can never corrupt each other's write, and a shared file a
/// third party installed concurrently with *different* data is never
/// silently overwritten.
///
/// Writes `credentials` to a unique temporary file in `shared`'s directory
/// first (mode `0600`, via [`write_credentials`]), then links it into
/// place with [`fs::hard_link`]. Unlike [`fs::rename`], `hard_link` fails
/// with [`std::io::ErrorKind::AlreadyExists`] instead of silently
/// replacing a destination that already exists, so exactly one caller
/// across every racing process gets to create `shared`. Every other
/// caller re-reads what is now there: identical data is a benign
/// double-migration (`Ok`); different data is a genuine conflict this
/// function refuses to resolve by picking a winner (`Err`, naming only
/// the path, never token values). The temporary file is removed in every
/// case (best-effort).
///
/// # Errors
///
/// Returns an error when the temporary file cannot be written, when a
/// concurrently installed `shared` cannot be read back afterward, or when
/// it holds account/session data that disagrees with `credentials`.
fn install_shared_credentials(
    shared: &Path,
    credentials: &StoredCredentials,
) -> Result<(), String> {
    // pid + nanosecond timestamp + a per-process monotonic counter: the
    // counter is what actually guarantees uniqueness — two threads in the
    // same process racing this function can land on the same
    // `SystemTime::now()` nanosecond reading under a coarser OS clock, and
    // a name collision there would let one thread's temp-file cleanup
    // delete the file another thread is about to `hard_link` from.
    static TMP_SUFFIX: AtomicU64 = AtomicU64::new(0);
    let file_name = shared
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("credentials.json"));
    let tmp_path = shared.with_file_name(format!(
        "{}.tmp-{}-{}-{}",
        file_name.to_string_lossy(),
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos()),
        TMP_SUFFIX.fetch_add(1, Ordering::Relaxed)
    ));

    write_credentials(&tmp_path, credentials)?;

    let outcome = match fs::hard_link(&tmp_path, shared) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            match read_credentials(shared)? {
                Some(installed) if &installed == credentials => Ok(()),
                Some(_) => Err(format!(
                    "another process concurrently installed {} with different \
                     account/session data; migration never overwrites a conflicting shared \
                     credential — resolve manually",
                    shared.display()
                )),
                None => Err(format!(
                    "{} was created concurrently and is no longer readable; retry",
                    shared.display()
                )),
            }
        }
        Err(error) => Err(format!("could not install {}: {error}", shared.display())),
    };

    let _ = fs::remove_file(&tmp_path);
    outcome
}

/// Removes a legacy file this process has just migrated into `shared`.
/// `NotFound` is treated as success: it means a process racing the same
/// migration already won and removed it first, which is the same end
/// state this call was trying to reach — so it must not turn an already
/// successful migration into a spurious startup error.
///
/// # Errors
///
/// Returns an error for any removal failure other than the file already
/// being gone.
fn remove_migrated_legacy_file(shared: &Path, path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "migrated legacy credentials to {} but could not remove {}: {error}",
            shared.display(),
            path.display()
        )),
    }
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

// Creates (or truncates) the file with mode 0600 set at open time, on
// Unix: no window where the session token is readable under the ambient
// umask before a follow-up chmod narrows it. Also re-applies 0600
// explicitly after opening: `.mode(0o600)` on `OpenOptions` only governs
// permissions at *creation* time, so a file that already existed with
// wider permissions (hand-created, restored from a backup, or written by
// another tool) would otherwise keep its old, looser mode across every
// rewrite.
#[cfg(unix)]
fn open_restricted(path: &Path) -> Result<fs::File, String> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| format!("could not open {}: {error}", path.display()))?;
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|error| {
            format!(
                "could not restrict permissions on {}: {error}",
                path.display()
            )
        })?;
    Ok(file)
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
    use std::sync::Mutex;

    // `resolve_default_credentials_path`/`env_session` read process-global
    // env vars (`MEMORY_ENGINE_HOME`, `MEMORY_ENGINE_ACCOUNT_ID`,
    // `MEMORY_ENGINE_SESSION_TOKEN`); the default test harness runs tests in
    // parallel threads, so any test that sets/removes one of these must
    // serialize against every other test that does. This lock is that
    // serialization point.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

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
    fn write_credentials_narrows_permissions_on_a_pre_existing_file() {
        let dir = tempdir("preexisting-mode");
        let path = dir.join("credentials.json");
        fs::write(&path, "{}").expect("seed pre-existing file");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644))
                .expect("loosen permissions to simulate a hand-created file");
        }

        let credentials = StoredCredentials {
            base_url: DEFAULT_BASE_URL.to_owned(),
            account_id: "acct_demo".to_owned(),
            session_token: "secret".to_owned(),
        };
        write_credentials(&path, &credentials).expect("write credentials");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
            assert_eq!(
                mode, 0o600,
                "pre-existing looser permissions must be narrowed before the token is rewritten"
            );
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
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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

    #[test]
    fn migration_is_a_no_op_when_no_legacy_files_exist() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempdir("migrate-none");
        env::set_var("MEMORY_ENGINE_HOME", &home);

        let resolved = resolve_default_credentials_path().expect("no legacy files is not an error");
        env::remove_var("MEMORY_ENGINE_HOME");

        assert_eq!(resolved, home.join("credentials.json"));
        assert!(
            !resolved.exists(),
            "must not create a file when there is nothing to migrate"
        );
    }

    #[test]
    fn migration_moves_a_single_legacy_review_file_into_the_shared_default() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempdir("migrate-review-only");
        env::set_var("MEMORY_ENGINE_HOME", &home);

        let legacy_path = home.join("review").join("credentials.json");
        let credentials = StoredCredentials {
            base_url: DEFAULT_BASE_URL.to_owned(),
            account_id: "acct_legacy_review".to_owned(),
            session_token: "legacy-review-token".to_owned(),
        };
        write_credentials(&legacy_path, &credentials).expect("seed legacy review file");

        let resolved = resolve_default_credentials_path().expect("migration succeeds");
        env::remove_var("MEMORY_ENGINE_HOME");

        assert_eq!(resolved, home.join("credentials.json"));
        assert!(
            !legacy_path.exists(),
            "the legacy file must be removed once migrated"
        );
        let migrated = read_credentials(&resolved)
            .expect("read migrated")
            .expect("present");
        assert_eq!(migrated, credentials);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&resolved)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn migration_moves_a_single_legacy_mcp_file_into_the_shared_default() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempdir("migrate-mcp-only");
        env::set_var("MEMORY_ENGINE_HOME", &home);

        let legacy_path = home.join("mcp").join("credentials.json");
        let credentials = StoredCredentials {
            base_url: DEFAULT_BASE_URL.to_owned(),
            account_id: "acct_legacy_mcp".to_owned(),
            session_token: "legacy-mcp-token".to_owned(),
        };
        write_credentials(&legacy_path, &credentials).expect("seed legacy mcp file");

        let resolved = resolve_default_credentials_path().expect("migration succeeds");
        env::remove_var("MEMORY_ENGINE_HOME");

        assert_eq!(resolved, home.join("credentials.json"));
        assert!(
            !legacy_path.exists(),
            "the legacy file must be removed once migrated"
        );
        let migrated = read_credentials(&resolved)
            .expect("read migrated")
            .expect("present");
        assert_eq!(migrated, credentials);
    }

    #[test]
    fn migration_reconciles_multiple_legacy_files_that_agree() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempdir("migrate-agree");
        env::set_var("MEMORY_ENGINE_HOME", &home);

        let review_path = home.join("review").join("credentials.json");
        let mcp_path = home.join("mcp").join("credentials.json");
        let credentials = StoredCredentials {
            base_url: DEFAULT_BASE_URL.to_owned(),
            account_id: "acct_shared_already".to_owned(),
            session_token: "same-token-both-sides".to_owned(),
        };
        write_credentials(&review_path, &credentials).expect("seed review file");
        write_credentials(&mcp_path, &credentials).expect("seed mcp file");

        let resolved =
            resolve_default_credentials_path().expect("migration succeeds when legacy files agree");
        env::remove_var("MEMORY_ENGINE_HOME");

        assert!(!review_path.exists());
        assert!(!mcp_path.exists());
        let migrated = read_credentials(&resolved)
            .expect("read migrated")
            .expect("present");
        assert_eq!(migrated, credentials);
    }

    #[test]
    fn migration_fails_explicitly_on_conflicting_legacy_files_naming_only_paths() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempdir("migrate-conflict");
        env::set_var("MEMORY_ENGINE_HOME", &home);

        let review_path = home.join("review").join("credentials.json");
        let mcp_path = home.join("mcp").join("credentials.json");
        let review_credentials = StoredCredentials {
            base_url: DEFAULT_BASE_URL.to_owned(),
            account_id: "acct_review_side".to_owned(),
            session_token: "review-side-secret-token".to_owned(),
        };
        let mcp_credentials = StoredCredentials {
            base_url: DEFAULT_BASE_URL.to_owned(),
            account_id: "acct_mcp_side".to_owned(),
            session_token: "mcp-side-secret-token".to_owned(),
        };
        write_credentials(&review_path, &review_credentials).expect("seed review file");
        write_credentials(&mcp_path, &mcp_credentials).expect("seed mcp file");

        let error = resolve_default_credentials_path()
            .expect_err("conflicting legacy files must fail explicitly");
        env::remove_var("MEMORY_ENGINE_HOME");

        assert!(error.contains(&review_path.display().to_string()));
        assert!(error.contains(&mcp_path.display().to_string()));
        assert!(
            !error.contains("review-side-secret-token") && !error.contains("mcp-side-secret-token"),
            "the conflict error must name paths only, never token values: {error}"
        );
        assert!(
            review_path.exists() && mcp_path.exists(),
            "conflicting legacy files must be left in place, not silently deleted"
        );
        assert!(
            !home.join("credentials.json").exists(),
            "no shared file must be written on an unresolved conflict"
        );
    }

    #[test]
    fn migration_is_skipped_once_the_shared_file_exists() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempdir("migrate-skip");
        env::set_var("MEMORY_ENGINE_HOME", &home);

        let shared_path = home.join("credentials.json");
        let shared_credentials = StoredCredentials {
            base_url: DEFAULT_BASE_URL.to_owned(),
            account_id: "acct_current".to_owned(),
            session_token: "current-token".to_owned(),
        };
        write_credentials(&shared_path, &shared_credentials).expect("seed shared file");

        let legacy_path = home.join("review").join("credentials.json");
        let stale_credentials = StoredCredentials {
            base_url: DEFAULT_BASE_URL.to_owned(),
            account_id: "acct_stale".to_owned(),
            session_token: "stale-token".to_owned(),
        };
        write_credentials(&legacy_path, &stale_credentials).expect("seed stale legacy file");

        let resolved = resolve_default_credentials_path()
            .expect("an existing shared file short-circuits migration");
        env::remove_var("MEMORY_ENGINE_HOME");

        assert_eq!(resolved, shared_path);
        let current = read_credentials(&resolved)
            .expect("read shared")
            .expect("present");
        assert_eq!(
            current, shared_credentials,
            "an existing shared file must not be overwritten by a stale legacy file"
        );
        assert!(
            legacy_path.exists(),
            "an ignored legacy file is left alone, not deleted, once the shared file already exists"
        );
    }

    #[test]
    fn concurrent_shared_install_from_agreeing_data_leaves_exactly_one_file() {
        let dir = tempdir("concurrent-install-agree");
        let shared = dir.join("credentials.json");
        let credentials = StoredCredentials {
            base_url: DEFAULT_BASE_URL.to_owned(),
            account_id: "acct_concurrent".to_owned(),
            session_token: "concurrent-token".to_owned(),
        };

        let thread_count = 8;
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(thread_count));
        let handles: Vec<_> = (0..thread_count)
            .map(|_| {
                let shared = shared.clone();
                let credentials = credentials.clone();
                let barrier = std::sync::Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    install_shared_credentials(&shared, &credentials)
                })
            })
            .collect();
        let results: Vec<Result<(), String>> = handles
            .into_iter()
            .map(|handle| handle.join().expect("thread must not panic"))
            .collect();

        assert!(
            results.iter().all(std::result::Result::is_ok),
            "every racer installing identical data must succeed, never spuriously fail: {results:?}"
        );

        let installed = read_credentials(&shared)
            .expect("read installed shared file")
            .expect("shared file must exist after the race");
        assert_eq!(installed, credentials);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&shared)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "the one surviving shared file must stay 0600");
        }

        let leftover_tmp_files = fs::read_dir(&dir)
            .expect("read dir")
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp-"))
            .count();
        assert_eq!(
            leftover_tmp_files, 0,
            "every racer's temporary install file must be cleaned up"
        );
    }

    #[test]
    fn concurrent_shared_install_with_conflicting_data_never_overwrites_the_winner() {
        let dir = tempdir("concurrent-install-conflict");
        let shared = dir.join("credentials.json");
        let credentials_a = StoredCredentials {
            base_url: DEFAULT_BASE_URL.to_owned(),
            account_id: "acct_a".to_owned(),
            session_token: "token-a-secret".to_owned(),
        };
        let credentials_b = StoredCredentials {
            base_url: DEFAULT_BASE_URL.to_owned(),
            account_id: "acct_b".to_owned(),
            session_token: "token-b-secret".to_owned(),
        };

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handle_a = {
            let shared = shared.clone();
            let credentials = credentials_a.clone();
            let barrier = std::sync::Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                install_shared_credentials(&shared, &credentials)
            })
        };
        let handle_b = {
            let shared = shared.clone();
            let credentials = credentials_b.clone();
            let barrier = std::sync::Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                install_shared_credentials(&shared, &credentials)
            })
        };

        let result_a = handle_a.join().expect("thread a must not panic");
        let result_b = handle_b.join().expect("thread b must not panic");
        let results = [&result_a, &result_b];

        let oks = results.iter().filter(|result| result.is_ok()).count();
        let errs = results.iter().filter(|result| result.is_err()).count();
        assert_eq!(
            (oks, errs),
            (1, 1),
            "exactly one racer must win a conflicting install and the other must fail \
             explicitly, never silently pick a winner: a={result_a:?} b={result_b:?}"
        );

        let error = results
            .iter()
            .find_map(|result| result.as_ref().err())
            .expect("one side must have failed");
        assert!(
            error.contains(&shared.display().to_string()),
            "the conflict error must name the shared path: {error}"
        );
        assert!(
            !error.contains("token-a-secret") && !error.contains("token-b-secret"),
            "the conflict error must never contain token values: {error}"
        );

        let installed = read_credentials(&shared)
            .expect("read installed shared file")
            .expect("shared file must exist");
        assert!(
            installed == credentials_a || installed == credentials_b,
            "the installed file must be exactly one racer's data, never a mix: {installed:?}"
        );
    }

    #[test]
    fn concurrent_first_launch_migration_never_fails_spuriously_and_removes_legacy_files() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = tempdir("migrate-concurrent");
        env::set_var("MEMORY_ENGINE_HOME", &home);

        let review_path = home.join("review").join("credentials.json");
        let mcp_path = home.join("mcp").join("credentials.json");
        let credentials = StoredCredentials {
            base_url: DEFAULT_BASE_URL.to_owned(),
            account_id: "acct_concurrent_launch".to_owned(),
            session_token: "concurrent-launch-token".to_owned(),
        };
        write_credentials(&review_path, &credentials).expect("seed review file");
        write_credentials(&mcp_path, &credentials).expect("seed mcp file");

        let thread_count = 6;
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(thread_count));
        let handles: Vec<_> = (0..thread_count)
            .map(|_| {
                let barrier = std::sync::Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    resolve_default_credentials_path()
                })
            })
            .collect();
        let results: Vec<Result<PathBuf, String>> = handles
            .into_iter()
            .map(|handle| handle.join().expect("thread must not panic"))
            .collect();
        env::remove_var("MEMORY_ENGINE_HOME");

        assert!(
            results.iter().all(std::result::Result::is_ok),
            "two clients racing a genuinely agreeing first launch must never fail spuriously: \
             {results:?}"
        );

        let shared_path = home.join("credentials.json");
        assert!(
            results
                .iter()
                .all(|result| result.as_ref().unwrap() == &shared_path),
            "every racer must resolve to the same one shared path"
        );

        let installed = read_credentials(&shared_path)
            .expect("read shared")
            .expect("shared file must exist after the race");
        assert_eq!(installed, credentials);
        assert!(
            !review_path.exists(),
            "the review legacy file must eventually be removed"
        );
        assert!(
            !mcp_path.exists(),
            "the mcp legacy file must eventually be removed"
        );
    }
}
