//! Opus adversarial review — "can this dashboard lie?" Each test below
//! demonstrates a way the rendered floor claims a healthier picture than the
//! underlying observation supports. These are EXPECTED TO FAIL against the
//! current code; each asserts the honest behaviour, not the current one.

use chrono::{DateTime, Duration, Utc};
use foundry_core::agents::AgentIngestObserver;
use foundry_core::bay::BayMap;
use foundry_core::eventlog::EventLog;
use foundry_core::health::{CapabilitySet, ObserverHealth};
use foundry_core::heartbeat::HeartbeatObserver;
use foundry_core::observer::{Observer, CAP_ROUTINES, CAP_SESSIONS};
use foundry_core::persist::{self, LoadOutcome};
use foundry_core::reducer::{MachineStatus, StateStore};
use foundry_core::render::render_floor;
use foundry_core::schema::{
    EntityRef, EntityType, Event, EventKind, Fidelity, Metrics, StationState,
};
use foundry_core::sign;
use foundry_core::transport::{
    AgentBundle, FileTransportPublisher, FileTransportReceiver, Publisher, SignedBundle,
};
use std::collections::BTreeMap;

fn base_event(ts: DateTime<Utc>, source: &str, kind: EventKind, entity: EntityRef) -> Event {
    Event {
        ts,
        source: source.into(),
        kind,
        entity,
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
        ttl_secs: None,
        next_run_at: None,
        enabled: None,
    }
}

fn working_session_event(id: &str, ts: DateTime<Utc>) -> Event {
    let mut ev = base_event(
        ts,
        "remote_claude",
        EventKind::SessionObserved,
        EntityRef::new(EntityType::Session, id),
    );
    ev.session_id = Some(id.into());
    ev.state = Some(StationState::Working);
    ev.label = Some("shipping the thing".into());
    ev.metrics = Metrics {
        elapsed_ms: Some(5_000),
        ..Default::default()
    };
    ev.ttl_secs = Some(120);
    ev
}

/// F-1 (HIGH): restart replay re-dates a day-old event log as live.
///
/// `main.rs` replays `events_since(snapshot.last_seq)` with `Utc::now()` as
/// the poll clock, so ancient logged observations are folded in as if they
/// had just been observed: `restored` is cleared, `pipeline.restored_at` is
/// cleared, and a session that was WORKING yesterday renders WORKING today
/// with no `(restored)` tag — from a process that has observed nothing.
#[test]
fn restart_replay_of_stale_event_log_must_not_render_working() {
    let dir = tempfile::tempdir().unwrap();
    let long_ago = Utc::now() - Duration::hours(26);

    // A prior process observed s1 WORKING a day ago, logged it, snapshotted.
    let mut log = EventLog::new(dir.path(), 100, 30).unwrap();
    let mut store = StateStore::new();
    let ev = working_session_event("s1", long_ago);
    store.apply_events(std::slice::from_ref(&ev), long_ago);
    log.append(std::slice::from_ref(&ev)).unwrap();
    // Snapshot taken here...
    persist::save_snapshot(dir.path(), &store, log.last_seq(), long_ago).unwrap();
    // ...and one more observation lands after it, so restart replays it.
    let ev2 = working_session_event("s1", long_ago + Duration::seconds(30));
    log.append(&[ev2]).unwrap();

    // --- new process, a day later: exactly main.rs's restore path ---
    let now = Utc::now();
    let log2 = EventLog::new(dir.path(), 100, 30).unwrap();
    let restored = match persist::load_snapshot(dir.path()) {
        LoadOutcome::Loaded(snap) => {
            let mut s = snap.store;
            persist::mark_restored(&mut s, snap.saved_at);
            let replay: Vec<Event> = log2
                .events_since(snap.last_seq)
                .unwrap()
                .into_iter()
                .map(|pe| pe.event)
                .collect();
            if !replay.is_empty() {
                s.apply_events(&replay, now);
            }
            s
        }
        _ => panic!("expected a loaded snapshot"),
    };

    let text = render_floor(&restored, now, &BayMap::new());
    assert_eq!(
        restored.sessions["s1"].displayed_state.value,
        StationState::StaleUnknown,
        "a day-old logged observation replayed at restart must stay STALE/UNKNOWN, not become WORKING"
    );
    assert!(
        restored.sessions["s1"].restored,
        "replaying an old log entry is not a fresh observation — `restored` must not be cleared"
    );
    assert!(
        restored.pipeline.restored_at.is_some(),
        "replaying the old event log must not clear the restored-snapshot marker"
    );
    assert!(
        !text.contains("WORKING"),
        "floor must not render WORKING off a replayed day-old log:\n{text}"
    );
}

/// F-2 (HIGH): a future-dated heartbeat line fabricates a fresh LAST OUTPUT.
///
/// `HeartbeatObserver` computes `elapsed_ms = (now - ts).max(0)`, so any line
/// whose `ts` is in the future (clock skew, or a writer that lies) clamps to
/// zero and the marquee reads `LAST OUTPUT: 0s ago`. `RemoteClaudeObserver`
/// explicitly rejects future timestamps as unparseable; this path does not.
#[test]
fn future_dated_heartbeat_must_not_fabricate_fresh_last_output() {
    let dir = tempfile::tempdir().unwrap();
    let fdir = dir.path().join(".foundry");
    std::fs::create_dir_all(&fdir).unwrap();
    let now = Utc::now();
    let future = now + Duration::hours(3);
    std::fs::write(
        fdir.join("events.jsonl"),
        format!(
            "{{\"component\":\"forward_capture\",\"event\":\"end\",\"status\":\"ok\",\"ts\":\"{}\"}}\n",
            future.to_rfc3339()
        ),
    )
    .unwrap();

    let mut hb = HeartbeatObserver::new(dir.path(), "SPORTS LAB");
    let events = hb.poll(now);
    let mut store = StateStore::new();
    store.apply_events(&events, now);
    let text = render_floor(&store, now, &BayMap::new());

    assert!(
        !text.contains("LAST OUTPUT: 0s ago"),
        "a heartbeat timestamped 3h in the FUTURE must not render as output 0s ago:\n{text}"
    );
}

/// F-3 (HIGH): an agent bundle with `seq == 0` can be replayed forever, so a
/// dead machine keeps rendering REACHABLE.
///
/// `verify_and_apply` guards replays with `if prior_seq > 0 && seq <= prior_seq`.
/// A bundle carrying seq 0 leaves `last_seq == 0`, so the guard never engages:
/// re-appending the identical signed line resurrects the agent row.
#[test]
fn replayed_seq_zero_bundle_must_not_resurrect_a_dead_machine() {
    let dir = tempfile::tempdir().unwrap();
    let t0 = Utc::now();
    let bundle = AgentBundle {
        agent_id: "laptop".into(),
        seq: 0,
        sent_at: t0,
        events: Vec::new(),
        health: Vec::new(),
    };
    let bundle_json = serde_json::to_string(&bundle).unwrap();
    let signed = SignedBundle {
        sig_hex: sign::sign_hex(b"secret", bundle_json.as_bytes()),
        bundle_json,
        key_id: "k1".into(),
    };

    let mut publisher = FileTransportPublisher::new(dir.path(), "laptop").unwrap();
    publisher.publish(signed.clone()).unwrap();

    let mut keyring = BTreeMap::new();
    keyring.insert("k1".to_string(), b"secret".to_vec());
    let mut ingest = AgentIngestObserver::new(
        Box::new(FileTransportReceiver::new(dir.path())),
        keyring,
        120,
    );
    ingest.poll(t0);
    assert_eq!(ingest.machines(t0)[0].status, MachineStatus::Reachable);

    // The agent process dies. 200s later somebody (or a retrying transport)
    // re-appends the exact same envelope — a byte-identical replay.
    let later = t0 + Duration::seconds(200);
    publisher.publish(signed).unwrap();
    ingest.poll(later);

    let rows = ingest.machines(later);
    assert_eq!(
        rows[0].status,
        MachineStatus::Unreachable,
        "a byte-identical replayed bundle must not make a dead machine REACHABLE again"
    );
}

/// F-4 (HIGH): "pipeline verified" survives a degraded, blind session observer.
///
/// `pipeline_verified` only refuses when a real observer is fully `Down`
/// (3 consecutive failures). Two failures leaves it `Degraded` with an empty
/// capability set — totally blind — yet the top line still reads
/// "THE FOUNDRY — LIVE (pipeline verified)".
#[test]
fn degraded_blind_session_observer_must_not_read_pipeline_verified() {
    let now = Utc::now();
    let mut store = StateStore::new();
    let canary = base_event(
        now,
        "synthetic_canary",
        EventKind::Heartbeat,
        EntityRef::new(EntityType::Project, "__pipeline__"),
    );
    store.apply_events(&[canary], now);

    let mut health = ObserverHealth::new("remote_claude");
    health.record_success(now, CapabilitySet::from_iter([CAP_SESSIONS, CAP_ROUTINES]));
    health.record_failure("bridge timeout");
    health.record_failure("bridge timeout");
    assert!(health.capabilities.0.is_empty(), "blind this poll");

    store.apply_observer_health(&health, now, Some(CAP_SESSIONS), Some(CAP_ROUTINES));
    let text = render_floor(&store, now, &BayMap::new());

    assert!(
        !store.pipeline_verified(now, 300),
        "the only sessions observer confirmed nothing this poll — pipeline is not verified"
    );
    assert!(
        !text.contains("pipeline verified"),
        "floor must not claim a verified pipeline while its session observer is blind:\n{text}"
    );
}

/// F-5 (HIGH): any non-canary source can mint the canary heartbeat.
///
/// `apply_events` sets `pipeline.last_canary_at` for EVERY `Heartbeat` event
/// regardless of source, and clears `pipeline.restored_at` for any non-canary
/// source. So a remote agent bundle (or any observer) emitting a Heartbeat
/// both certifies the local poll loop as alive and un-marks a restored floor.
#[test]
fn foreign_heartbeat_must_not_certify_the_local_canary() {
    let now = Utc::now();
    let mut store = StateStore::new();
    store.pipeline.restored_at = Some(now - Duration::hours(2));

    let foreign = base_event(
        now,
        "canary@laptop", // an agent-forwarded event, not our in-process canary
        EventKind::Heartbeat,
        EntityRef::new(EntityType::Project, "__pipeline__"),
    );
    store.apply_events(&[foreign], now);

    let text = render_floor(&store, now, &BayMap::new());
    assert!(
        store.pipeline.last_canary_at.is_none(),
        "only the in-process synthetic_canary may set last_canary_at"
    );
    assert!(
        !store.pipeline_verified(now, 300),
        "a foreign heartbeat must not verify the local pipeline"
    );
    assert!(
        !text.contains("pipeline verified"),
        "floor must not read LIVE off a heartbeat minted by another machine:\n{text}"
    );
}

/// F-6 (MED): the marquee's "next:" routine ignores staleness.
///
/// The ROUTINES section correctly tags a routine `[STALE]` once its observer
/// loses the routines capability, but the marquee's soonest-due lookup filters
/// only on `enabled` — so an unverified, frozen schedule is still advertised
/// as the estate's next scheduled run.
#[test]
fn marquee_next_routine_must_not_advertise_a_stale_schedule() {
    let now = Utc::now();
    let mut store = StateStore::new();
    let mut ev = base_event(
        now,
        "remote_claude",
        EventKind::RoutineScheduled,
        EntityRef::new(EntityType::Routine, "trig_1"),
    );
    ev.label = Some("nightly capture".into());
    ev.enabled = Some(true);
    ev.next_run_at = Some(now + Duration::hours(1));
    store.apply_events(&[ev], now);

    // The observer loses the routines capability entirely.
    let mut health = ObserverHealth::new("remote_claude");
    health.record_success(now, CapabilitySet::from_iter([CAP_SESSIONS, CAP_ROUTINES]));
    health.record_success(now, CapabilitySet::from_iter([CAP_SESSIONS]));
    store.apply_observer_health(&health, now, Some(CAP_SESSIONS), Some(CAP_ROUTINES));
    assert!(
        store.routines["trig_1"].stale,
        "precondition: routine stale"
    );

    let text = render_floor(&store, now, &BayMap::new());
    let marquee = text
        .lines()
        .find(|l| l.contains("routine(s) overdue"))
        .unwrap_or_default()
        .to_string();
    assert!(
        !marquee.contains("nightly capture"),
        "a stale (unverified this poll) routine must not be advertised as the next run:\n{text}"
    );
}

/// F-7 (MED): the agents observer reports itself Healthy even when its
/// transport directory does not exist.
///
/// `AgentIngestObserver::poll` unconditionally calls `record_success`, so a
/// misconfigured/deleted `--agents-dir` — from which nothing can ever be
/// received — renders a green `[HEALTHY] agents` row on the OBSERVERS list.
#[test]
fn agents_observer_must_not_be_healthy_with_an_unreadable_transport() {
    let now = Utc::now();
    let missing = std::path::PathBuf::from("/nonexistent/foundry-agents-dir");
    let mut ingest = AgentIngestObserver::new(
        Box::new(FileTransportReceiver::new(missing)),
        BTreeMap::new(),
        120,
    );
    ingest.poll(now);
    assert_ne!(
        ingest.health().status,
        foundry_core::health::ObserverStatus::Healthy,
        "an unreadable agent transport directory must not read as a HEALTHY observer"
    );
}
