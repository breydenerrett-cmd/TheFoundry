//! §18 "lying dashboard" red-team suite, first cut, run against the REAL
//! RemoteClaudeObserver + StateStore + render pipeline (not just the reducer
//! unit tests) — this is what actually gets exercised for Phase 3's truth
//! gate. Every case asserts the floor does NOT render as healthy while it is
//! actually blind, stale, or fed garbage.

use chrono::{DateTime, Duration, Utc};
use foundry_core::observer::{Observer, RemoteClaudeObserver};
use foundry_core::reducer::StateStore;
use foundry_core::render::render_floor;
use std::fs;
use std::path::Path;

fn write(dir: &Path, name: &str, contents: &str) {
    fs::write(dir.join(name), contents).unwrap();
}

fn poll_once(feed_dir: &Path, now: DateTime<Utc>) -> (StateStore, String) {
    let mut obs = RemoteClaudeObserver::new(feed_dir);
    let events = obs.poll(now);
    let mut store = StateStore::new();
    store.apply_events(&events, now);
    store.apply_observer_health(obs.health(), now);
    let text = render_floor(&store, now);
    (store, text)
}

/// 1. Sever the network / kill the watcher's data source: feed dir exists but
///    is empty (no files at all — as if the bridge never ran).
#[test]
fn empty_feed_dir_never_renders_healthy() {
    let dir = tempfile::tempdir().unwrap();
    let now = Utc::now();
    let (store, text) = poll_once(dir.path(), now);

    assert!(store.sessions.is_empty(), "no sessions should be fabricated from nothing");
    assert!(!store.pipeline_verified(now, 300), "no canary ever fired in this poll — must not be verified");
    assert_ne!(
        store.observer_health["remote_claude"].status,
        foundry_core::health::ObserverStatus::Healthy,
        "a totally empty feed dir must not read as a healthy observer"
    );
    assert!(!text.to_lowercase().contains("pipeline verified"), "must not claim verified with zero data");
}

/// 2. Corrupted JSONL/JSON mid-line: the file exists but is truncated/broken.
#[test]
fn corrupted_json_degrades_instead_of_crashing_or_faking_data() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "list_sessions.json", "{\"data\": [ { \"id\": \"session_x\", TRUNCATED GARBAGE");
    let now = Utc::now();
    let (store, _text) = poll_once(dir.path(), now);

    // Must not panic (test reaching here already proves that) and must not
    // silently invent a session from garbage.
    assert!(store.sessions.is_empty());
    assert_ne!(
        store.observer_health["remote_claude"].status,
        foundry_core::health::ObserverStatus::Healthy,
        "unparseable JSON must not read as a healthy observer"
    );
}

/// 3. Unknown enum value the schema has never seen — must degrade to
///    StaleUnknown/Unknown fidelity, never silently default to a healthy state.
#[test]
fn unknown_status_bucket_becomes_stale_unknown_not_silently_healthy() {
    let dir = tempfile::tempdir().unwrap();
    let now = Utc::now();
    let body = serde_json::json!({
        "data": [{
            "id": "session_weird",
            "title": "future API shape",
            "environment_kind": "anthropic_cloud",
            "connection_status": "connected",
            "status_bucket": "SESSION_STATUS_BUCKET_TELEPORTED", // does not exist in our enum mapping
            "configured_model": "claude-sonnet-9",
            "updated_at": now.to_rfc3339(),
        }]
    });
    write(dir.path(), "list_sessions.json", &body.to_string());
    let (store, text) = poll_once(dir.path(), now);

    let rec = &store.sessions["session_weird"];
    assert_eq!(rec.displayed_state.value, foundry_core::schema::StationState::StaleUnknown);
    assert_eq!(rec.displayed_state.fidelity, foundry_core::schema::Fidelity::Unknown);
    assert!(!text.contains("[        WORKING]"), "an unknown state must never render as WORKING");
}

/// 4. "Freeze the clock": a session the API still reports as WORKING, but
///    whose own last-activity timestamp is hours old — must render HUNG, not
///    a still-fresh-looking WORKING station.
#[test]
fn frozen_clock_working_session_renders_hung_not_still_fresh() {
    let dir = tempfile::tempdir().unwrap();
    let now = Utc::now();
    let three_hours_ago = now - Duration::hours(3);
    let body = serde_json::json!({
        "data": [{
            "id": "session_frozen",
            "title": "looks busy, isn't",
            "environment_kind": "bridge",
            "connection_status": "connected",
            "status_bucket": "SESSION_STATUS_BUCKET_WORKING",
            "configured_model": "claude-sonnet-5",
            "updated_at": three_hours_ago.to_rfc3339(),
        }]
    });
    write(dir.path(), "list_sessions.json", &body.to_string());
    let (store, text) = poll_once(dir.path(), now);

    let rec = &store.sessions["session_frozen"];
    assert_eq!(rec.displayed_state.value, foundry_core::schema::StationState::Hung);
    assert_eq!(rec.displayed_state.fidelity, foundry_core::schema::Fidelity::Inferred);
    assert!(text.contains("HUNG"));
}

/// 5. Empty session list (as opposed to no file at all): the API responded,
///    there is genuinely nothing running. Must render as "no sessions", not
///    stay frozen on stale prior data or fabricate anything.
#[test]
fn genuinely_empty_session_list_is_not_confused_with_a_dead_feed() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "list_sessions.json", r#"{"data": []}"#);
    let now = Utc::now();
    let (store, text) = poll_once(dir.path(), now);

    assert!(store.sessions.is_empty());
    // Capability WAS supplied (the file parsed successfully, just empty) —
    // this is a real "nothing running" observation, distinct from case 1's
    // "we couldn't observe at all".
    assert_eq!(
        store.observer_health["remote_claude"].status,
        foundry_core::health::ObserverStatus::Healthy
    );
    assert!(text.contains("no sessions observed"));
}

/// 6. A disconnected bridge machine (computer_unreachable) must render
///    StaleUnknown, never IDLE — IDLE implies "nothing to do", which is a
///    different and more comforting claim than "we can't see this machine".
#[test]
fn unreachable_bridge_machine_is_stale_not_idle() {
    let dir = tempfile::tempdir().unwrap();
    let now = Utc::now();
    let body = serde_json::json!({
        "data": [{
            "id": "session_dark",
            "title": "laptop is closed",
            "environment_kind": "bridge",
            "connection_status": "disconnected",
            "status_bucket": "SESSION_STATUS_BUCKET_REVIEW_READY",
            "configured_model": "claude-opus-5",
            "updated_at": now.to_rfc3339(),
            "last_init_error": {"error_kind": "computer_unreachable", "recoverable": true}
        }]
    });
    write(dir.path(), "list_sessions.json", &body.to_string());
    let (store, _text) = poll_once(dir.path(), now);

    let rec = &store.sessions["session_dark"];
    assert_eq!(rec.displayed_state.value, foundry_core::schema::StationState::StaleUnknown);
    assert_ne!(rec.displayed_state.value, foundry_core::schema::StationState::Idle);
}

/// 7. Observer going Down mid-run must degrade PREVIOUSLY healthy sessions —
///    they must not stay frozen at their last-good ("Working"/"Idle") value
///    forever, which would look permanently fine even though we've lost the
///    ability to check.
#[test]
fn observer_down_after_a_healthy_poll_degrades_existing_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let now = Utc::now();
    let body = serde_json::json!({
        "data": [{
            "id": "session_ok_then_blind",
            "title": "fine, until we can't see it anymore",
            "environment_kind": "anthropic_cloud",
            "connection_status": "connected",
            "status_bucket": "SESSION_STATUS_BUCKET_WORKING",
            "configured_model": "claude-sonnet-5",
            "updated_at": now.to_rfc3339(),
        }]
    });
    write(dir.path(), "list_sessions.json", &body.to_string());

    let mut obs = RemoteClaudeObserver::new(dir.path());
    let mut store = StateStore::new();

    let events1 = obs.poll(now);
    store.apply_events(&events1, now);
    store.apply_observer_health(obs.health(), now);
    assert_eq!(store.sessions["session_ok_then_blind"].displayed_state.value, foundry_core::schema::StationState::Working);

    // Now the feed goes dark (file removed — network severed).
    fs::remove_file(dir.path().join("list_sessions.json")).unwrap();
    let later = now + Duration::seconds(10);
    let events2 = obs.poll(later);
    store.apply_events(&events2, later);
    store.apply_observer_health(obs.health(), later);

    let rec = &store.sessions["session_ok_then_blind"];
    assert_eq!(rec.displayed_state.value, foundry_core::schema::StationState::StaleUnknown, "must not still read Working once the observer is Down");
}

/// 8. The pipeline-verified canary must actually gate the top-level claim —
///    a poll with real session data but no canary event must still show
///    UNVERIFIED, since the canary is what proves the whole loop is alive,
///    not just that one file happened to parse.
#[test]
fn healthy_looking_snapshot_without_canary_is_still_unverified() {
    let dir = tempfile::tempdir().unwrap();
    let now = Utc::now();
    let body = serde_json::json!({
        "data": [{
            "id": "session_fine",
            "title": "looks great",
            "environment_kind": "anthropic_cloud",
            "connection_status": "connected",
            "status_bucket": "SESSION_STATUS_BUCKET_WORKING",
            "configured_model": "claude-sonnet-5",
            "updated_at": now.to_rfc3339(),
        }]
    });
    write(dir.path(), "list_sessions.json", &body.to_string());
    let (store, text) = poll_once(dir.path(), now); // no SyntheticCanary polled here on purpose

    assert!(!store.pipeline_verified(now, 300));
    assert!(text.contains("UNVERIFIED"), "a plausible-looking snapshot must not read as verified without the canary");
}
