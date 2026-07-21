//! Invite-beta waitlist storage: the smallest durable record of who asked to
//! be notified, kept independent of the study-store adapter (`storage.rs`) so
//! this slice cannot destabilize the account/session/study contract.
//!
//! File-store only. Production runs on Postgres
//! (`memory_engine_persistence_postgres::PostgresStudyStore::waitlist_*`,
//! dispatched from `AccountRegistry`); this module exists so local
//! development and the fast test lane can exercise the full join/list/
//! invite/delete/audit contract without a live database. Every mutation
//! also appends a line to an append-only audit log beside the entries file,
//! mirroring the Postgres `memory_engine_waitlist_audit_log` table.

use std::{
    collections::BTreeMap,
    fs,
    io::Write as _,
    path::{Path, PathBuf},
};

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

/// One append-only audit-log line for a waitlist transition. Mirrors the
/// Postgres `memory_engine_waitlist_audit_log` table's columns.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WaitlistAuditEvent {
    email: String,
    event: String,
    occurred_at_ms: i64,
}

fn lock_path(store_path: &Path) -> std::path::PathBuf {
    store_path.with_file_name("_waitlist.lock")
}

fn audit_log_path(store_path: &Path) -> PathBuf {
    store_path.with_file_name("_waitlist_audit.jsonl")
}

/// Append one line to the audit log. Append-only: no prior line is ever
/// rewritten or removed, including by [`delete`] — a deleted address stays
/// provable in the log after its operational row is gone.
fn append_audit_event(
    store_path: &Path,
    email: &str,
    event: &'static str,
    now_ms: i64,
) -> Result<(), ApiFailure> {
    let line = serde_json::to_string(&WaitlistAuditEvent {
        email: email.to_owned(),
        event: event.to_owned(),
        occurred_at_ms: now_ms,
    })
    .map_err(|error| ApiFailure::internal(format!("waitlist audit encode: {error}")))?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(audit_log_path(store_path))
        .map_err(|error| ApiFailure::internal(format!("waitlist audit open: {error}")))?;
    writeln!(file, "{line}")
        .map_err(|error| ApiFailure::internal(format!("waitlist audit write: {error}")))
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
    save(store_path, &entries)?;
    append_audit_event(store_path, email, "joined", now_ms)
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

/// Mark one entry invited, idempotently: a repeat call for an
/// already-invited entry leaves `invited_at_ms` unchanged and appends no
/// further audit row. Returns `None` when no entry matches the email.
///
/// # Errors
///
/// Returns an API failure when the shared file state is busy or storage I/O
/// fails.
pub(crate) fn mark_invited(
    store_path: &Path,
    email: &str,
    now_ms: i64,
) -> Result<Option<WaitlistEntry>, ApiFailure> {
    let _lock = file_lock::acquire_blocking(&lock_path(store_path))?;
    let mut entries = load(store_path)?;
    let Some(entry) = entries.get_mut(email) else {
        return Ok(None);
    };
    let already_invited = entry.invited_at_ms.is_some();
    if !already_invited {
        entry.invited_at_ms = Some(now_ms);
    }
    let result = entries.get(email).cloned();
    save(store_path, &entries)?;
    if !already_invited {
        append_audit_event(store_path, email, "invited", now_ms)?;
    }
    Ok(result)
}

/// Delete one entry from the operational table, appending a `deleted`
/// audit-log entry. Returns `false` when no entry matched.
///
/// # Errors
///
/// Returns an API failure when the shared file state is busy or storage I/O
/// fails.
pub(crate) fn delete(store_path: &Path, email: &str, now_ms: i64) -> Result<bool, ApiFailure> {
    let _lock = file_lock::acquire_blocking(&lock_path(store_path))?;
    let mut entries = load(store_path)?;
    let removed = entries.remove(email).is_some();
    if removed {
        save(store_path, &entries)?;
        append_audit_event(store_path, email, "deleted", now_ms)?;
    }
    Ok(removed)
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

    fn read_audit_events(store_path: &Path) -> Vec<WaitlistAuditEvent> {
        let raw = fs::read_to_string(audit_log_path(store_path)).expect("read audit log");
        raw.lines()
            .map(|line| serde_json::from_str(line).expect("audit line decodes"))
            .collect()
    }

    #[test]
    fn mark_invited_transitions_once_and_is_idempotent() {
        let path = temp_store_path("invite");
        join(&path, "learner@example.com", "first-run", 1_000).expect("join");

        let invited = mark_invited(&path, "learner@example.com", 2_000)
            .expect("mark invited")
            .expect("entry exists");
        assert_eq!(invited.invited_at_ms, Some(2_000));

        let invited_again = mark_invited(&path, "learner@example.com", 3_000)
            .expect("mark invited again")
            .expect("entry exists");
        assert_eq!(invited_again.invited_at_ms, Some(2_000));
    }

    #[test]
    fn mark_invited_is_none_for_an_unknown_email() {
        let path = temp_store_path("invite-unknown");
        assert_eq!(
            mark_invited(&path, "stranger@example.com", 1_000).expect("mark invited"),
            None
        );
    }

    #[test]
    fn delete_removes_the_row_but_keeps_the_audit_trail() {
        let path = temp_store_path("delete");
        join(&path, "learner@example.com", "first-run", 1_000).expect("join");
        mark_invited(&path, "learner@example.com", 2_000).expect("mark invited");

        assert!(delete(&path, "learner@example.com", 3_000).expect("delete"));
        assert!(list(&path).expect("list").is_empty());
        assert!(!delete(&path, "learner@example.com", 4_000).expect("second delete"));

        let events = read_audit_events(&path);
        assert_eq!(
            events
                .iter()
                .map(|event| (event.email.as_str(), event.event.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("learner@example.com", "joined"),
                ("learner@example.com", "invited"),
                ("learner@example.com", "deleted"),
            ]
        );
    }

    #[test]
    fn audit_log_is_append_only_across_repeat_joins() {
        let path = temp_store_path("audit-append-only");
        join(&path, "learner@example.com", "first-run", 1_000).expect("first join");
        join(&path, "learner@example.com", "first-run", 2_000).expect("second join");
        join(&path, "learner@example.com", "first-run", 3_000).expect("third join");

        let events = read_audit_events(&path);
        assert_eq!(events.len(), 3);
        assert!(events.iter().all(|event| event.event == "joined"));
        assert_eq!(
            events
                .iter()
                .map(|event| event.occurred_at_ms)
                .collect::<Vec<_>>(),
            vec![1_000, 2_000, 3_000]
        );
    }
}
