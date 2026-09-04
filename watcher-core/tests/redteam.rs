//! §18 "lying dashboard" red-team suite, first cut, run against the REAL
//! RemoteClaudeObserver + StateStore + render pipeline (not just the reducer
//! unit tests) — this is what actually gets exercised for Phase 3's truth
//! gate. Every case asserts the floor does NOT render as healthy while it is
//! actually blind, stale, or fed garbage.

use chrono::{DateTime, Duration, Utc};
use foundry_core::observer::{Observer, RemoteClaudeObserver, SyntheticCanary};
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
    store.apply_observer_health(obs.health(), now, Some(foundry_core::observer::CAP_SESSIONS), Some(foundry_core::observer::CAP_ROUTINES));
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
    store.apply_observer_health(obs.health(), now, Some(foundry_core::observer::CAP_SESSIONS), Some(foundry_core::observer::CAP_ROUTINES));
    assert_eq!(store.sessions["session_ok_then_blind"].displayed_state.value, foundry_core::schema::StationState::Working);

    // Now the feed goes dark (file removed — network severed).
    fs::remove_file(dir.path().join("list_sessions.json")).unwrap();
    let later = now + Duration::seconds(10);
    let events2 = obs.poll(later);
    store.apply_events(&events2, later);
    store.apply_observer_health(obs.health(), later, Some(foundry_core::observer::CAP_SESSIONS), Some(foundry_core::observer::CAP_ROUTINES));

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

// --- adversarial review findings (Opus-tier review, applied to the full pipeline) ---

/// Finding #1: the canary alone proved nothing. A fully DOWN remote_claude
/// observer, with the in-process canary still ticking along fine, must NOT
/// render "pipeline verified" — that claim has to reflect the real data
/// sources, not just that the poll loop itself is alive.
#[test]
fn dead_remote_observer_is_unverified_even_with_a_live_canary() {
    let dir = tempfile::tempdir().unwrap();
    let now = Utc::now();
    // No list_sessions.json / list_triggers.json at all -> remote_claude
    // gets zero capabilities on every poll -> Down after 3 tries.
    let mut remote = RemoteClaudeObserver::new(dir.path());
    let mut canary = SyntheticCanary::new();
    let mut store = StateStore::new();
    for i in 0..3 {
        let t = now + Duration::seconds(i);
        let mut events = remote.poll(t);
        events.extend(canary.poll(t));
        store.apply_events(&events, t);
        store.apply_observer_health(remote.health(), t, Some(foundry_core::observer::CAP_SESSIONS), Some(foundry_core::observer::CAP_ROUTINES));
        store.apply_observer_health(canary.health(), t, None, None);
    }
    assert_eq!(remote.health().status, foundry_core::health::ObserverStatus::Down);

    let final_now = now + Duration::seconds(3);
    let text = render_floor(&store, final_now);
    assert!(!store.pipeline_verified(final_now, 300), "must not be verified while remote_claude is Down");
    assert!(text.contains("UNVERIFIED"), "banner must say UNVERIFIED, not LIVE: {text}");
    assert!(!text.contains("LIVE (pipeline verified)"));
}

/// Finding #3: losing ONLY the sessions capability (routines still parse
/// fine) must degrade sessions specifically, WITHOUT falsely also marking
/// still-healthy routines as broken.
#[test]
fn partial_capability_loss_degrades_only_the_lost_capability() {
    let dir = tempfile::tempdir().unwrap();
    let now = Utc::now();
    let sessions_body = serde_json::json!({"data": [{
        "id": "session_partial",
        "environment_kind": "anthropic_cloud",
        "connection_status": "connected",
        "status_bucket": "SESSION_STATUS_BUCKET_WORKING",
        "updated_at": now.to_rfc3339(),
    }]});
    let triggers_body = serde_json::json!({"data": [{
        "id": "trig_partial", "name": "hourly thing", "enabled": true,
        "next_run_at": (now + Duration::hours(1)).to_rfc3339(),
    }]});
    write(dir.path(), "list_sessions.json", &sessions_body.to_string());
    write(dir.path(), "list_triggers.json", &triggers_body.to_string());

    let mut remote = RemoteClaudeObserver::new(dir.path());
    let mut store = StateStore::new();
    let events1 = remote.poll(now);
    store.apply_events(&events1, now);
    store.apply_observer_health(remote.health(), now, Some(foundry_core::observer::CAP_SESSIONS), Some(foundry_core::observer::CAP_ROUTINES));
    assert_eq!(store.sessions["session_partial"].displayed_state.value, foundry_core::schema::StationState::Working);
    assert!(!store.routines["trig_partial"].stale);

    // Sessions capability vanishes; routines capability still fine.
    fs::remove_file(dir.path().join("list_sessions.json")).unwrap();
    let later = now + Duration::seconds(10);
    let events2 = remote.poll(later);
    store.apply_events(&events2, later);
    store.apply_observer_health(remote.health(), later, Some(foundry_core::observer::CAP_SESSIONS), Some(foundry_core::observer::CAP_ROUTINES));

    assert_eq!(
        store.sessions["session_partial"].displayed_state.value,
        foundry_core::schema::StationState::StaleUnknown,
        "sessions must degrade once their capability is lost"
    );
    assert!(
        !store.routines["trig_partial"].stale,
        "routines must NOT be falsely marked stale just because sessions broke — routines capability is still fine"
    );
}

/// Finding #3 (fabricated zero): one malformed session record must not lose
/// the entire batch and silently report "zero sessions" as if that were a
/// confirmed fact.
#[test]
fn one_malformed_session_record_does_not_nuke_the_whole_batch() {
    let dir = tempfile::tempdir().unwrap();
    let now = Utc::now();
    // needs_action is a bool here where the schema expects a string — this
    // one record is malformed, the other is perfectly fine.
    let body = serde_json::json!({"data": [
        {
            "id": "session_good",
            "environment_kind": "bridge",
            "connection_status": "connected",
            "status_bucket": "SESSION_STATUS_BUCKET_WORKING",
            "updated_at": now.to_rfc3339(),
        },
        {
            "id": "session_bad",
            "environment_kind": "bridge",
            "connection_status": "connected",
            "status_bucket": "SESSION_STATUS_BUCKET_BLOCKED",
            "post_turn_summary": {"needs_action": true}, // wrong type, should be a string
            "updated_at": now.to_rfc3339(),
        }
    ]});
    write(dir.path(), "list_sessions.json", &body.to_string());
    let (store, text) = poll_once(dir.path(), now);

    assert!(store.sessions.contains_key("session_good"), "the one good record in the batch must still come through: {text}");
    assert_eq!(
        store.observer_health["remote_claude"].status,
        foundry_core::health::ObserverStatus::Healthy,
        "a batch with one bad record among good ones is a real partial success, not a total failure"
    );
}

/// Finding #4: routines must degrade too when their capability is lost —
/// not just sessions. A 2-poll-old routine snapshot must not keep rendering
/// a confident ON SCHEDULE/OVERDUE verdict.
#[test]
fn routines_go_stale_when_routines_capability_is_lost() {
    let dir = tempfile::tempdir().unwrap();
    let now = Utc::now();
    let body = serde_json::json!({"data": [{
        "id": "trig_x", "name": "hourly thing", "enabled": true,
        "next_run_at": (now - Duration::hours(2)).to_rfc3339(), // overdue
    }]});
    write(dir.path(), "list_triggers.json", &body.to_string());

    let mut remote = RemoteClaudeObserver::new(dir.path());
    let mut store = StateStore::new();
    let events1 = remote.poll(now);
    store.apply_events(&events1, now);
    store.apply_observer_health(remote.health(), now, Some(foundry_core::observer::CAP_SESSIONS), Some(foundry_core::observer::CAP_ROUTINES));
    assert!(store.routines["trig_x"].overdue);
    assert!(!store.routines["trig_x"].stale);
    let text1 = render_floor(&store, now);
    assert!(text1.contains("[OVERDUE]"));

    fs::remove_file(dir.path().join("list_triggers.json")).unwrap();
    let later = now + Duration::hours(2);
    let events2 = remote.poll(later);
    store.apply_events(&events2, later);
    store.apply_observer_health(remote.health(), later, Some(foundry_core::observer::CAP_SESSIONS), Some(foundry_core::observer::CAP_ROUTINES));

    assert!(store.routines["trig_x"].stale, "routine must be marked stale once its capability is lost");
    let text2 = render_floor(&store, later);
    assert!(text2.contains("[STALE]"), "a stale routine must render distinctly, not still confidently OVERDUE/ON SCHEDULE: {text2}");
}

/// Finding #4: an `enabled` field that's missing entirely (unknown, not
/// false) must NOT be treated as confidently active — the schema's own
/// documented contract is "unknown means unknown, not enabled".
#[test]
fn unknown_enabled_routine_does_not_render_as_confidently_active() {
    let dir = tempfile::tempdir().unwrap();
    let now = Utc::now();
    let body = serde_json::json!({"data": [{
        "id": "trig_unknown_enabled", "name": "mystery routine",
        "next_run_at": (now - Duration::hours(1)).to_rfc3339(),
        // "enabled" field omitted entirely.
    }]});
    write(dir.path(), "list_triggers.json", &body.to_string());
    let (store, text) = poll_once(dir.path(), now);

    let rec = &store.routines["trig_unknown_enabled"];
    assert!(!rec.enabled, "unknown enabled-state must default to NOT confidently active");
    assert!(!rec.overdue, "must not claim OVERDUE (implies active+stuck) when we don't even know if it's enabled");
    assert!(!text.contains("[ON SCHEDULE]"), "must not render a confident green verdict for an unknown enabled-state: {text}");
}

/// Finding #5: a session that vanishes from the snapshot must get a 60s
/// "fading" grace state (visible, distinctly tagged) before it's excluded
/// from the main list — it must never just silently disappear with no trace,
/// and even past the fade window a footer must still acknowledge it happened.
#[test]
fn vanished_session_fades_then_folds_with_a_trace_not_silently() {
    let dir = tempfile::tempdir().unwrap();
    let now = Utc::now();
    let body = serde_json::json!({"data": [{
        "id": "session_will_vanish",
        "environment_kind": "anthropic_cloud",
        "connection_status": "connected",
        "status_bucket": "SESSION_STATUS_BUCKET_WORKING",
        "updated_at": now.to_rfc3339(),
    }]});
    write(dir.path(), "list_sessions.json", &body.to_string());

    let mut remote = RemoteClaudeObserver::new(dir.path());
    let mut store = StateStore::new();
    let events1 = remote.poll(now);
    store.apply_events(&events1, now);
    store.apply_observer_health(remote.health(), now, Some(foundry_core::observer::CAP_SESSIONS), Some(foundry_core::observer::CAP_ROUTINES));
    assert!(store.sessions.contains_key("session_will_vanish"));

    // The session vanishes from the next snapshot entirely.
    write(dir.path(), "list_sessions.json", r#"{"data": []}"#);
    let just_after = now + Duration::seconds(5);
    let events2 = remote.poll(just_after);
    store.apply_events(&events2, just_after);
    store.apply_observer_health(remote.health(), just_after, Some(foundry_core::observer::CAP_SESSIONS), Some(foundry_core::observer::CAP_ROUTINES));

    assert!(store.sessions["session_will_vanish"].gone);
    let text_fading = render_floor(&store, just_after);
    assert!(
        text_fading.contains("session_will_vanish") && text_fading.contains("FADING"),
        "within the 60s grace window the vanished session must still be visible, distinctly tagged: {text_fading}"
    );

    // Well past the 60s fade window.
    let long_after = now + Duration::seconds(300);
    let text_ended = render_floor(&store, long_after);
    assert!(
        !text_ended.contains("session_will_vanish"),
        "past the fade window it should no longer clutter the main list"
    );
    assert!(
        text_ended.to_lowercase().contains("ended this run"),
        "but it must still leave a trace in a footer, never just vanish silently: {text_ended}"
    );
}

/// Finding #6: redact.rs existed but was never actually called anywhere in
/// the pipeline — a secret, an absolute path, and an email embedded in a
/// raw session title/status text flowed straight through to the rendered
/// output and would have hit the on-disk event log unredacted. This proves
/// the wiring end-to-end through the REAL observer, not just redact.rs's
/// own unit tests in isolation.
#[test]
fn secrets_in_raw_session_text_are_redacted_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let now = Utc::now();
    let body = serde_json::json!({"data": [{
        "id": "session_leaky",
        "title": "deploy with sk-abcdefghij1234567890 from /home/user/secretproj (contact brey@example.com)",
        "environment_kind": "bridge",
        "connection_status": "connected",
        "status_bucket": "SESSION_STATUS_BUCKET_WORKING",
        "updated_at": now.to_rfc3339(),
    }]});
    write(dir.path(), "list_sessions.json", &body.to_string());
    let (store, text) = poll_once(dir.path(), now);

    let rec = &store.sessions["session_leaky"];
    let label = rec.label.as_ref().map(|f| f.value.as_str()).unwrap_or("");
    assert!(!label.contains("sk-abcdefghij1234567890"), "API key must be redacted from the stored label: {label}");
    assert!(!label.contains("/home/user/secretproj"), "absolute path must be redacted: {label}");
    assert!(!label.contains("brey@example.com"), "email must be redacted: {label}");
    assert!(!text.contains("sk-abcdefghij1234567890"), "must not reach the rendered text either: {text}");
    assert!(!text.contains("brey@example.com"));
}

/// Regression: TWO different observers can legitimately source SessionObserved
/// records under DIFFERENT capability names (remote_claude uses "sessions",
/// local_claude uses "local_sessions"). apply_observer_health must check each
/// observer against ITS OWN capability name — hardcoding one name (as an
/// earlier version of this fix did) falsely degraded the second observer's
/// perfectly healthy sessions, because it never reports the first observer's
/// capability name.
#[test]
fn distinct_observers_with_different_session_capability_names_dont_shadow_each_other() {
    use foundry_core::health::ObserverHealth;
    use foundry_core::reducer::StateStore;
    use foundry_core::schema::{EntityRef, EntityType, EventKind, Fidelity, Metrics, StationState};

    let now = Utc::now();
    let mut store = StateStore::new();

    let remote_event = foundry_core::schema::Event {
        ts: now,
        source: "remote_claude".into(),
        kind: EventKind::SessionObserved,
        entity: EntityRef::new(EntityType::Session, "session_remote"),
        project_id: None,
        session_id: Some("session_remote".into()),
        model: None, model_current: None, model_last_served: None, effort: None,
        state: Some(StationState::Working),
        label: None, detail: None,
        fidelity: Fidelity::Observed,
        metrics: Metrics::default(), ttl_secs: None, next_run_at: None, enabled: None,
    };
    let mut local_event = remote_event.clone();
    local_event.source = "local_claude".into();
    local_event.entity = EntityRef::new(EntityType::Session, "sid_local");
    local_event.session_id = Some("sid_local".into());
    local_event.fidelity = Fidelity::Inferred;

    store.apply_events(&[remote_event, local_event], now);

    let mut remote_health = ObserverHealth::new("remote_claude");
    remote_health.record_success(now, foundry_core::health::CapabilitySet::from_iter(["sessions"]));
    let mut local_health = ObserverHealth::new("local_claude");
    local_health.record_success(now, foundry_core::health::CapabilitySet::from_iter(["local_sessions"]));

    store.apply_observer_health(&remote_health, now, Some("sessions"), Some("routines"));
    store.apply_observer_health(&local_health, now, Some("local_sessions"), None);

    assert_eq!(
        store.sessions["session_remote"].displayed_state.value, StationState::Working,
        "remote-sourced session must stay healthy — its own capability (sessions) is present"
    );
    assert_eq!(
        store.sessions["sid_local"].displayed_state.value, StationState::Working,
        "local-sourced session must ALSO stay healthy — its own capability (local_sessions) is present, even though remote_claude's capability name differs"
    );
}
