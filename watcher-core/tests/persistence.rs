//! S-01 persistent state with honest restart — proves the truth rule end to
//! end: a snapshot loaded at startup must never render as live/healthy
//! until something in THIS process actually re-observes it.

use chrono::{DateTime, Duration, Utc};
use foundry_core::bay::BayMap;
use foundry_core::eventlog::EventLog;
use foundry_core::persist::{self, LoadOutcome};
use foundry_core::reducer::StateStore;
use foundry_core::render::render_floor;
use foundry_core::schema::{
    EntityRef, EntityType, Event, EventKind, Fidelity, Metrics, StationState,
};

fn session_event(id: &str, state: StationState, ts: DateTime<Utc>, ttl_secs: Option<i64>) -> Event {
    Event {
        ts,
        source: "remote_claude".into(),
        kind: EventKind::SessionObserved,
        entity: EntityRef::new(EntityType::Session, id),
        project_id: None,
        session_id: Some(id.into()),
        model: Some("claude-sonnet-5".into()),
        model_current: None,
        model_last_served: None,
        effort: None,
        state: Some(state),
        label: Some("doing real work".into()),
        detail: Some("anthropic_cloud".into()),
        fidelity: Fidelity::Observed,
        metrics: Metrics::default(),
        ttl_secs,
        next_run_at: None,
        enabled: None,
    }
}

fn canary_event(ts: DateTime<Utc>) -> Event {
    Event {
        ts,
        source: "synthetic_canary".into(),
        kind: EventKind::Heartbeat,
        entity: EntityRef::new(EntityType::Project, "__pipeline__"),
        project_id: None,
        session_id: None,
        model: None,
        model_current: None,
        model_last_served: None,
        effort: None,
        state: None,
        label: None,
        detail: None,
        fidelity: Fidelity::Observed,
        metrics: Metrics::default(),
        ttl_secs: Some(120),
        next_run_at: None,
        enabled: None,
    }
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip the CSI sequence: ESC [ ... letter
            if chars.peek() == Some(&'[') {
                chars.next();
                for c2 in chars.by_ref() {
                    if c2.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// (a) Save a snapshot with a WORKING session, load it in a fresh process,
/// render it: the marquee is UNVERIFIED and the session renders
/// STALE/UNKNOWN "(restored)" — never WORKING.
#[test]
fn restored_snapshot_renders_unverified_and_session_stale_not_working() {
    let dir = tempfile::tempdir().unwrap();
    let now = Utc::now();

    let mut store = StateStore::new();
    store.apply_events(
        &[
            session_event("s_working", StationState::Working, now, Some(3600)),
            canary_event(now),
        ],
        now,
    );
    assert_eq!(
        store.sessions["s_working"].displayed_state.value,
        StationState::Working
    );
    persist::save_snapshot(dir.path(), &store, 2, now).unwrap();

    // Fresh "process": load it back and apply the truth-rule marking exactly
    // as main.rs does on restart.
    let later = now + Duration::minutes(5);
    let restored_store = match persist::load_snapshot(dir.path()) {
        LoadOutcome::Loaded(snap) => {
            let mut s = snap.store;
            persist::mark_restored(&mut s, snap.saved_at);
            s
        }
        _ => panic!("expected a loaded snapshot"),
    };

    let text = render_floor(&restored_store, later, &BayMap::new());
    let clean = strip_ansi(&text);
    assert!(
        clean.contains("UNVERIFIED (restored snapshot"),
        "must show the restored-snapshot UNVERIFIED banner: {clean}"
    );
    assert!(
        clean.contains("awaiting fresh observations"),
        "banner must explain why: {clean}"
    );
    assert!(!clean.contains("LIVE (pipeline verified)"));

    let rec = &restored_store.sessions["s_working"];
    assert_eq!(
        rec.displayed_state.value,
        StationState::StaleUnknown,
        "a restored WORKING session must never render WORKING again until re-observed"
    );
    assert!(rec.restored);
    assert!(clean.contains("s_working"));
    assert!(
        clean.contains("(restored)"),
        "the restored session must be visibly tagged: {clean}"
    );
    assert!(
        !clean.contains("[         WORKING]"),
        "must never render a restored record as a live WORKING station: {clean}"
    );
    assert!(!restored_store.pipeline_verified(later, 300));
}

/// (b) After a fresh event for the restored session (plus a fresh canary and
/// a fresh real-observer event), the session renders normally again and the
/// pipeline verifies.
#[test]
fn fresh_event_after_restore_clears_stale_tag_and_reverifies_pipeline() {
    let dir = tempfile::tempdir().unwrap();
    let now = Utc::now();

    let mut store = StateStore::new();
    store.apply_events(
        &[session_event(
            "s_working",
            StationState::Working,
            now,
            Some(3600),
        )],
        now,
    );
    persist::save_snapshot(dir.path(), &store, 1, now).unwrap();

    let later = now + Duration::minutes(1);
    let mut restored_store = match persist::load_snapshot(dir.path()) {
        LoadOutcome::Loaded(snap) => {
            let mut s = snap.store;
            persist::mark_restored(&mut s, snap.saved_at);
            s
        }
        _ => panic!("expected a loaded snapshot"),
    };
    assert!(!restored_store.pipeline_verified(later, 300));

    // A real observer re-observes the session AND the canary ticks — both
    // required for pipeline_verified per the mission brief.
    let fresh_time = later + Duration::seconds(1);
    restored_store.apply_events(
        &[
            session_event("s_working", StationState::Working, fresh_time, Some(3600)),
            canary_event(fresh_time),
        ],
        fresh_time,
    );

    let rec = &restored_store.sessions["s_working"];
    assert!(
        !rec.restored,
        "restored flag must clear on a fresh observation"
    );
    assert_eq!(rec.displayed_state.value, StationState::Working);

    assert!(
        restored_store.pipeline_verified(fresh_time, 300),
        "canary + a real observer's fresh event must reverify the pipeline"
    );
    let text = strip_ansi(&render_floor(&restored_store, fresh_time, &BayMap::new()));
    assert!(text.contains("LIVE (pipeline verified)"));
    assert!(!text.contains("(restored)"));
}

/// (c) Sequence numbers keep climbing across a restart: write 3 events,
/// "restart" (new EventLog over the same directory), write 1 more -> seq 4.
#[test]
fn sequence_numbers_continue_across_restart() {
    let dir = tempfile::tempdir().unwrap();
    let now = Utc::now();

    {
        let mut log = EventLog::new(dir.path(), 100, 30).unwrap();
        for i in 0..3 {
            log.append(&[session_event(
                "s1",
                StationState::Working,
                now + Duration::seconds(i),
                None,
            )])
            .unwrap();
        }
        assert_eq!(log.last_seq(), 2, "seqs 0,1,2 assigned so far");
    }

    // "Restart": a brand new EventLog over the same on-disk directory must
    // recover where the last process left off, not reset to 0.
    let mut log2 = EventLog::new(dir.path(), 100, 30).unwrap();
    assert_eq!(log2.next_seq(), 3);
    log2.append(&[session_event(
        "s1",
        StationState::Idle,
        now + Duration::seconds(10),
        None,
    )])
    .unwrap();
    assert_eq!(
        log2.last_seq(),
        3,
        "the 4th event ever written gets seq 3 (0-indexed)"
    );

    // events_since is exclusive of `since_seq` itself (it means "everything
    // AFTER the snapshot's last_seq") — seq 0 is excluded here on purpose.
    let all = log2.events_since(0).unwrap();
    assert_eq!(
        all.len(),
        3,
        "seqs 1,2,3 — seq 0 is excluded by the exclusive bound"
    );
    assert_eq!(all.last().unwrap().seq, 3);
}

/// (d) A session past its TTL renders STALE, even though nothing marked it
/// gone and no observer went Down.
#[test]
fn ttl_expiry_renders_stale_without_a_gone_signal_or_observer_down() {
    let mut store = StateStore::new();
    let t0 = Utc::now();
    store.apply_events(
        &[session_event("s_ttl", StationState::Working, t0, Some(30))],
        t0,
    );
    assert_eq!(
        store.sessions["s_ttl"].displayed_state.value,
        StationState::Working
    );

    // No fresh event arrives; time passes well beyond the 30s TTL.
    let later = t0 + Duration::seconds(90);
    store.apply_events(&[], later);

    let rec = &store.sessions["s_ttl"];
    assert_eq!(
        rec.displayed_state.value,
        StationState::StaleUnknown,
        "an observation older than its TTL must expire to STALE/UNKNOWN"
    );
    assert_eq!(rec.displayed_state.fidelity, Fidelity::Unknown);
    let text = strip_ansi(&render_floor(&store, later, &BayMap::new()));
    assert!(text.contains("STALE/UNKNOWN"));
}

/// (e) A secret embedded in a label must never reach snapshot.json or the
/// JSONL log on disk, even by way of the persistence path.
#[test]
fn secret_never_reaches_snapshot_or_jsonl_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let now = Utc::now();

    let mut ev = session_event("s_leaky", StationState::Working, now, Some(3600));
    // Mirrors what the redaction boundary hands the reducer: already
    // scrubbed. This test's job is to prove the PERSIST path (snapshot +
    // JSONL) doesn't reintroduce or otherwise leak the raw secret once it's
    // in the store/log — not to re-derive redact.rs's own behavior (that's
    // redteam.rs's job).
    ev.label = Some(foundry_core::redact::redact_field(
        "token sk-abcdefghij1234567890 from /home/user/secretproj (brey@example.com)",
    ));

    let mut store = StateStore::new();
    let mut log = EventLog::new(dir.path(), 100, 30).unwrap();
    store.apply_events(std::slice::from_ref(&ev), now);
    log.append(std::slice::from_ref(&ev)).unwrap();
    persist::save_snapshot(dir.path(), &store, log.last_seq(), now).unwrap();

    let secret = "sk-abcdefghij1234567890";
    let path_leak = "/home/user/secretproj";
    let email_leak = "brey@example.com";

    let snapshot_raw = std::fs::read_to_string(dir.path().join("snapshot.json")).unwrap();
    assert!(
        !snapshot_raw.contains(secret),
        "secret leaked into snapshot.json"
    );
    assert!(
        !snapshot_raw.contains(path_leak),
        "path leaked into snapshot.json"
    );
    assert!(
        !snapshot_raw.contains(email_leak),
        "email leaked into snapshot.json"
    );

    let mut jsonl_found = false;
    for entry in std::fs::read_dir(dir.path()).unwrap().flatten() {
        let p = entry.path();
        if p.extension().map(|e| e == "jsonl").unwrap_or(false) {
            jsonl_found = true;
            let raw = std::fs::read_to_string(&p).unwrap();
            assert!(!raw.contains(secret), "secret leaked into {}", p.display());
            assert!(!raw.contains(path_leak), "path leaked into {}", p.display());
            assert!(
                !raw.contains(email_leak),
                "email leaked into {}",
                p.display()
            );
        }
    }
    assert!(
        jsonl_found,
        "expected at least one events-*.jsonl file to exist"
    );
}

/// (f) A corrupted snapshot.json must not crash startup: it is ignored with
/// a visible (Corrupted) outcome, nothing is restored, and the pipeline
/// stays unverified until fresh observations arrive — exactly like a
/// process that never had a snapshot at all.
#[test]
fn corrupted_snapshot_is_ignored_not_crashed_and_nothing_restored() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("snapshot.json"),
        "{ this is not valid json ]]]",
    )
    .unwrap();

    let outcome = persist::load_snapshot(dir.path());
    let store = match outcome {
        LoadOutcome::Corrupted(msg) => {
            assert!(
                !msg.is_empty(),
                "corruption message should say something useful"
            );
            StateStore::new()
        }
        _ => panic!("expected the corrupted snapshot to be reported as Corrupted"),
    };

    let now = Utc::now();
    assert!(
        store.sessions.is_empty(),
        "nothing should be restored from a corrupted snapshot"
    );
    assert!(!store.pipeline_verified(now, 300));
    let text = strip_ansi(&render_floor(&store, now, &BayMap::new()));
    assert!(!text.contains("LIVE (pipeline verified)"));
    assert!(!text.contains("(restored)"));
}
