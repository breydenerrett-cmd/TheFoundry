//! S-01: persistent state across restarts, honestly.
//!
//! A `Snapshot` is a point-in-time JSON dump of the `StateStore`, written to
//! `<log-dir>/snapshot.json` with an atomic temp-file-then-rename write so a
//! crash mid-write can never leave a half-written file behind for the next
//! startup to trip over. Paired with `eventlog::EventLog`'s seq-numbered
//! JSONL, a restart can load the snapshot and then replay only the events
//! that landed after it (`EventLog::events_since(snapshot.last_seq)`).
//!
//! JSON is deliberately the whole story here — SQLite is allowed by the
//! design brief but not required, and a JSON snapshot + JSONL event log is
//! sufficient for THE FOUNDRY's single-process, single-writer shape. If a
//! SQLite swap is ever warranted (e.g. the store grows too large to
//! reasonably serialize whole on every poll cycle), it belongs BEHIND this
//! same module — `save_snapshot`/`load_snapshot`'s signatures are the
//! contract the rest of the crate depends on, not the JSON format itself.
//!
//! The truth rule this module exists to uphold (see `mark_restored`): a
//! record whose current values came from a loaded snapshot, not a live
//! observation in this process, must never render as healthy. It is not
//! enough to just load the old state back in — every session/routine/check
//! gets tagged `restored`, and the pipeline itself is held UNVERIFIED until
//! a fresh event from a real (non-canary) observer proves the floor is
//! live again.

use crate::reducer::StateStore;
use crate::schema::{Fidelity, StationState};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const SCHEMA_VERSION: u32 = 1;
const SNAPSHOT_FILENAME: &str = "snapshot.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub schema_version: u32,
    pub saved_at: DateTime<Utc>,
    pub last_seq: u64,
    pub store: StateStore,
}

/// Outcome of trying to load a snapshot at startup. Distinguished from a
/// plain `Result` so callers can tell "nothing to restore, that's fine" (a
/// perfectly normal first run) apart from "something was there and it was
/// broken" (which must degrade honestly, not crash, and should say so).
pub enum LoadOutcome {
    /// No snapshot file at this path — a normal fresh start.
    Missing,
    /// A snapshot file existed but could not be parsed (corrupted, wrong
    /// shape, truncated write from a killed process, etc). Callers must NOT
    /// crash on this — start fresh instead, with a visible warning, exactly
    /// as if there were no snapshot at all.
    Corrupted(String),
    Loaded(Snapshot),
}

fn snapshot_path(dir: &Path) -> PathBuf {
    dir.join(SNAPSHOT_FILENAME)
}

/// Atomically writes `store` (plus its seq watermark) to
/// `<dir>/snapshot.json`: serialize to a temp file in the same directory,
/// flush+sync, then rename over the real path. The rename is what makes this
/// atomic on the filesystems THE FOUNDRY targets — a reader can never
/// observe a partially-written snapshot, only the previous complete one or
/// the new complete one.
pub fn save_snapshot(
    dir: &Path,
    store: &StateStore,
    last_seq: u64,
    now: DateTime<Utc>,
) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let snapshot = Snapshot {
        schema_version: SCHEMA_VERSION,
        saved_at: now,
        last_seq,
        store: store.clone(),
    };
    let json = serde_json::to_string_pretty(&snapshot)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    let tmp_path = dir.join(format!("{SNAPSHOT_FILENAME}.tmp.{}", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp_path)?;
        f.write_all(json.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp_path, snapshot_path(dir))?;
    Ok(())
}

/// Loads `<dir>/snapshot.json` if present. Never panics and never returns an
/// `Err` for "file missing" or "file corrupted" — both are ordinary
/// startup conditions the caller must handle by continuing with a fresh
/// `StateStore`, just distinguished so the caller can print an honest
/// message for the corrupted case.
pub fn load_snapshot(dir: &Path) -> LoadOutcome {
    let path = snapshot_path(dir);
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return LoadOutcome::Missing,
        Err(e) => return LoadOutcome::Corrupted(format!("could not read {}: {e}", path.display())),
    };
    match serde_json::from_str::<Snapshot>(&raw) {
        Ok(snapshot) => LoadOutcome::Loaded(snapshot),
        Err(e) => LoadOutcome::Corrupted(format!("could not parse {}: {e}", path.display())),
    }
}

/// S-01 truth rule, applied to a just-loaded snapshot's store, in place:
///
/// - Every session/routine/check is tagged `restored` + `restored_at`. Every
///   session's `displayed_state` is forced to `StaleUnknown`/`Unknown`
///   fidelity regardless of what it was doing when the snapshot was taken —
///   a session that was WORKING a minute or an hour ago is not still
///   WORKING just because the process restarted; nothing has re-observed it
///   yet in THIS process.
/// - Observer health is dropped entirely (not restored) — every observer
///   starts `Unverified` again, exactly as a genuinely fresh process would,
///   per the mission brief.
/// - `pipeline.restored_at` is set, which `StateStore::pipeline_verified`
///   checks and refuses to report verified while it is set — cleared only
///   once a fresh, non-canary-sourced event lands (see
///   `StateStore::apply_events`).
pub fn mark_restored(store: &mut StateStore, saved_at: DateTime<Utc>) {
    for rec in store.sessions.values_mut() {
        rec.restored = true;
        rec.restored_at = Some(saved_at);
        rec.displayed_state =
            crate::reducer::Field::new(StationState::StaleUnknown, saved_at, Fidelity::Unknown);
    }
    for rec in store.routines.values_mut() {
        rec.restored = true;
        rec.restored_at = Some(saved_at);
    }
    for rec in store.checks.values_mut() {
        rec.restored = true;
        rec.restored_at = Some(saved_at);
    }
    // Observer health is NOT restored — every observer starts Unverified.
    store.observer_health.clear();
    store.pipeline.restored_at = Some(saved_at);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reducer::StateStore;

    #[test]
    fn missing_snapshot_is_reported_as_missing_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(load_snapshot(dir.path()), LoadOutcome::Missing));
    }

    #[test]
    fn corrupted_snapshot_is_reported_not_panicked() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(SNAPSHOT_FILENAME), "{ not json at all").unwrap();
        assert!(matches!(
            load_snapshot(dir.path()),
            LoadOutcome::Corrupted(_)
        ));
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new();
        let now = Utc::now();
        save_snapshot(dir.path(), &store, 7, now).unwrap();
        match load_snapshot(dir.path()) {
            LoadOutcome::Loaded(snap) => {
                assert_eq!(snap.last_seq, 7);
                assert_eq!(snap.schema_version, SCHEMA_VERSION);
            }
            _ => panic!("expected a loaded snapshot"),
        }
    }

    #[test]
    fn save_is_atomic_no_tmp_file_left_behind() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new();
        save_snapshot(dir.path(), &store, 0, Utc::now()).unwrap();
        let leftover_tmp = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().contains(".tmp."));
        assert!(
            !leftover_tmp,
            "temp file must be renamed away, not left behind"
        );
    }
}
