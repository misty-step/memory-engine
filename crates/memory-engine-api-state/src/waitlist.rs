//! Invite-beta waitlist storage: the smallest durable record of who asked to
//! be notified, kept independent of the study-store adapter (`storage.rs`) so
//! this first slice cannot destabilize the account/session/study contract.
//!
//! File-store only for now. Production runs on Postgres and must not silently
//! accept a waitlist join it cannot persist durably — see
//! [`crate::AccountRegistry::join_waitlist`], which returns
//! [`crate::ApiFailure::service_unavailable`] rather than writing to the
//! ephemeral container filesystem when the registry is Postgres-backed.

use std::{collections::BTreeMap, fs, path::Path};

use serde::{Deserialize, Serialize};

use crate::{file_lock, write_atomic, ApiFailure};

/// One waitlist row: normalized email, an audit trail of when it joined and
/// last changed, the first-run surface it came from, and whether an operator
/// has since transitioned it to invited. No account, session, or generation
/// state is ever attached to this record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitlistEntry {
    pub email: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    /// Where the join happened, e.g. `"first-run"`. Submitting the form is
    /// the consent action; this field is the source half of the
    /// "consent/source metadata" the card asks for.
    pub source: String,
    pub invited_at_ms: Option<i64>,
}

fn lock_path(store_path: &Path) -> std::path::PathBuf {
    store_path.with_file_name("_waitlist.lock")
}

fn load(store_path: &Path) -> Result<BTreeMap<String, WaitlistEntry>, ApiFailure> {
    match fs::read(store_path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| ApiFailure::internal(format!("waitlist store decode: {error}"))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(error) => Err(ApiFailure::internal(format!(
            "waitlist store read: {error}"
        ))),
    }
}

fn save(store_path: &Path, entries: &BTreeMap<String, WaitlistEntry>) -> Result<(), ApiFailure> {
    let bytes = serde_json::to_vec(entries)
        .map_err(|error| ApiFailure::internal(format!("waitlist store encode: {error}")))?;
    write_atomic(store_path, &bytes)
        .map_err(|error| ApiFailure::internal(format!("waitlist store write: {error}")))
}

/// Record a join. Idempotent on normalized email: a repeat join only bumps
/// `updated_at_ms`, so the caller's response is identical whether this is the
/// first join or the tenth — no "already on the waitlist" branch that would
/// let a caller distinguish a fresh entry from an existing one.
///
/// # Errors
///
/// Returns an API failure when the shared file state is busy or storage I/O
/// fails.
pub(crate) fn join(
    store_path: &Path,
    email: &str,
    source: &str,
    now_ms: i64,
) -> Result<(), ApiFailure> {
    let _lock = file_lock::acquire_blocking(&lock_path(store_path))?;
    let mut entries = load(store_path)?;
    entries
        .entry(email.to_owned())
        .and_modify(|entry| entry.updated_at_ms = now_ms)
        .or_insert_with(|| WaitlistEntry {
            email: email.to_owned(),
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            source: source.to_owned(),
            invited_at_ms: None,
        });
    save(store_path, &entries)
}

/// List every entry, ordered by normalized email. Operator-only: the caller
/// (`AccountRegistry::list_waitlist`) gates this behind the admin token
/// before ever touching the store.
///
/// # Errors
///
/// Returns an API failure when the shared file state is busy or storage I/O
/// fails.
pub(crate) fn list(store_path: &Path) -> Result<Vec<WaitlistEntry>, ApiFailure> {
    let _lock = file_lock::acquire_blocking(&lock_path(store_path))?;
    Ok(load(store_path)?.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir()
            .join(format!(
                "memory-engine-waitlist-store-{name}-{}-{}",
                std::process::id(),
                rand::random::<u64>()
            ))
            .join("_waitlist.json")
    }

    #[test]
    fn join_is_idempotent_and_preserves_created_at() {
        let path = temp_store_path("idempotent");
        join(&path, "learner@example.com", "first-run", 1_000).expect("first join");
        join(&path, "learner@example.com", "first-run", 2_000).expect("second join");

        let entries = list(&path).expect("list");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].created_at_ms, 1_000);
        assert_eq!(entries[0].updated_at_ms, 2_000);
    }

    #[test]
    fn list_is_empty_for_a_store_that_was_never_written() {
        let path = temp_store_path("empty");
        assert!(list(&path).expect("list").is_empty());
    }
}
