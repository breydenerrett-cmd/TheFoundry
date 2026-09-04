//! Phase 4D M-01 end-to-end: agent -> `FileTransport` -> main-side ingest ->
//! render. Exercises the full authenticated pipeline the way the real
//! `foundry-agent` / `foundry` binaries wire it up, not just the reducer
//! unit tests already covered in `src/agents.rs`.

use chrono::{DateTime, Duration, Utc};
use foundry_core::agents::AgentIngestObserver;
use foundry_core::bay::BayMap;
use foundry_core::health::{CapabilitySet, ObserverHealth};
use foundry_core::observer::Observer;
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

fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
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

fn session_event(id: &str, agent_id: &str, label: &str, now: DateTime<Utc>) -> Event {
    Event {
        ts: now,
        source: format!("local_claude@{agent_id}"),
        kind: EventKind::SessionObserved,
        entity: EntityRef::new(EntityType::Session, id).with_parent("/repo"),
        project_id: None,
        session_id: Some(id.into()),
        model: None,
        model_current: None,
        model_last_served: None,
        effort: None,
        state: Some(StationState::Working),
        label: Some(label.to_string()),
        detail: Some(format!("local@{agent_id}")),
        fidelity: Fidelity::Inferred,
        metrics: Metrics::default(),
        ttl_secs: Some(60),
        next_run_at: None,
        enabled: None,
    }
}

fn make_bundle(
    agent_id: &str,
    seq: u64,
    sent_at: DateTime<Utc>,
    events: Vec<Event>,
) -> AgentBundle {
    let mut health = ObserverHealth::new("local_claude");
    health.record_success(sent_at, CapabilitySet::from_iter(["local_sessions"]));
    AgentBundle {
        agent_id: agent_id.to_string(),
        seq,
        sent_at,
        events,
        health: vec![health],
    }
}

fn sign_and_publish(
    publisher: &mut FileTransportPublisher,
    bundle: &AgentBundle,
    secret: &[u8],
    key_id: &str,
) {
    let bundle_json = serde_json::to_string(bundle).unwrap();
    let sig_hex = sign::sign_hex(secret, bundle_json.as_bytes());
    publisher
        .publish(SignedBundle {
            bundle_json,
            sig_hex,
            key_id: key_id.to_string(),
        })
        .unwrap();
}

fn keyring(key_id: &str, secret: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut m = BTreeMap::new();
    m.insert(key_id.to_string(), secret.to_vec());
    m
}

/// (a) agent -> FileTransport -> main ingest: sessions render with
/// `@<agent_id>` source and a MACHINES row.
#[test]
fn agent_events_flow_through_to_render_with_machine_row() {
    let dir = tempfile::tempdir().unwrap();
    let secret = b"correct-horse-battery-staple";
    let now = Utc::now();

    let mut publisher = FileTransportPublisher::new(dir.path(), "pc").unwrap();
    let bundle = make_bundle(
        "pc",
        1,
        now,
        vec![session_event("s1", "pc", "doing work", now)],
    );
    sign_and_publish(&mut publisher, &bundle, secret, "k1");

    let mut ingest = AgentIngestObserver::new(
        Box::new(FileTransportReceiver::new(dir.path())),
        keyring("k1", secret),
        120,
    );
    let events = ingest.poll(now);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].source, "local_claude@pc");

    let mut store = StateStore::new();
    store.apply_events(&events, now);
    store.apply_observer_health(ingest.health(), now, None, None);
    store.set_machines(ingest.machines(now));

    assert_eq!(store.sessions["s1"].source, "local_claude@pc");
    assert_eq!(store.machines["pc"].status, MachineStatus::Reachable);

    let text = strip_ansi(&render_floor(&store, now, &BayMap::new()));
    assert!(text.contains("local_claude@pc") || text.contains("[local@pc]"));
    assert!(text.contains("MACHINES"));
    assert!(text.contains("pc"));
}

/// (b) tampered bundle_json -> rejected, agent shows Degraded reason, no
/// sessions ingested.
#[test]
fn tampered_bundle_is_rejected_and_no_sessions_ingest() {
    let dir = tempfile::tempdir().unwrap();
    let secret = b"correct-horse-battery-staple";
    let now = Utc::now();

    let bundle = make_bundle(
        "pc",
        1,
        now,
        vec![session_event("s1", "pc", "doing work", now)],
    );
    let bundle_json = serde_json::to_string(&bundle).unwrap();
    let sig_hex = sign::sign_hex(secret, bundle_json.as_bytes());
    // Tamper with the bundle body AFTER signing — the signature no longer
    // matches, even though the JSON is still well-formed and still names
    // the same agent_id (so the rejection can still be labeled).
    let tampered_json = bundle_json.replace("\"doing work\"", "\"totally fabricated\"");

    let mut publisher = FileTransportPublisher::new(dir.path(), "pc").unwrap();
    publisher
        .publish(SignedBundle {
            bundle_json: tampered_json,
            sig_hex,
            key_id: "k1".to_string(),
        })
        .unwrap();

    let mut ingest = AgentIngestObserver::new(
        Box::new(FileTransportReceiver::new(dir.path())),
        keyring("k1", secret),
        120,
    );
    let events = ingest.poll(now);
    assert!(
        events.is_empty(),
        "a tampered bundle must not yield any events"
    );

    let rows = ingest.machines(now);
    assert_eq!(rows.len(), 1);
    // F-9 fix: a bundle whose SIGNATURE has not verified carries an
    // UNVERIFIED `agent_id` claim — attributing the rejection to that
    // claimed "pc" would let an unauthenticated writer fabricate an
    // arbitrary machine's row (e.g. paint over a real "pc" agent's status,
    // or invent one that never existed). It is bucketed under the fixed
    // "unverified" row instead; only a bundle whose signature actually
    // verifies (see `replayed_seq_is_rejected_end_to_end` below) has an
    // authenticated agent_id worth keying a row by.
    assert_eq!(rows[0].agent_id, foundry_core::agents::UNVERIFIED_BUCKET);
    assert_eq!(rows[0].status, MachineStatus::Unreachable);
    assert!(rows[0].reason.as_deref().unwrap().contains("signature"));

    let mut store = StateStore::new();
    store.apply_events(&events, now);
    store.set_machines(rows);
    assert!(store.sessions.is_empty());
    assert_eq!(
        store.machines[foundry_core::agents::UNVERIFIED_BUCKET].status,
        MachineStatus::Unreachable
    );
}

/// (c) wrong key -> rejected.
#[test]
fn wrong_key_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let now = Utc::now();

    let bundle = make_bundle("pc", 1, now, vec![session_event("s1", "pc", "work", now)]);
    let mut publisher = FileTransportPublisher::new(dir.path(), "pc").unwrap();
    // Signed with a key the agent thinks is right...
    sign_and_publish(&mut publisher, &bundle, b"attackers-guess", "k1");

    // ...but main's keyring holds the REAL secret under that same key_id.
    let mut ingest = AgentIngestObserver::new(
        Box::new(FileTransportReceiver::new(dir.path())),
        keyring("k1", b"the-real-secret"),
        120,
    );
    let events = ingest.poll(now);
    assert!(events.is_empty());
    let rows = ingest.machines(now);
    assert_eq!(rows[0].status, MachineStatus::Unreachable);
    assert!(rows[0].reason.as_deref().unwrap().contains("signature"));
}

/// (d) replayed seq -> rejected.
#[test]
fn replayed_seq_is_rejected_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let secret = b"shared-secret";
    let now = Utc::now();

    let mut publisher = FileTransportPublisher::new(dir.path(), "pc").unwrap();
    let bundle1 = make_bundle("pc", 3, now, vec![session_event("s1", "pc", "work", now)]);
    sign_and_publish(&mut publisher, &bundle1, secret, "k1");

    let mut ingest = AgentIngestObserver::new(
        Box::new(FileTransportReceiver::new(dir.path())),
        keyring("k1", secret),
        120,
    );
    let first_events = ingest.poll(now);
    assert_eq!(first_events.len(), 1);
    assert_eq!(ingest.machines(now)[0].status, MachineStatus::Reachable);

    // A second envelope re-using the SAME seq (a captured-and-replayed
    // publish) must be rejected even though the signature itself is valid.
    let later = now + Duration::seconds(5);
    let bundle2 = make_bundle(
        "pc",
        3,
        later,
        vec![session_event("s2", "pc", "replay", later)],
    );
    sign_and_publish(&mut publisher, &bundle2, secret, "k1");
    let second_events = ingest.poll(later);
    assert!(
        second_events.is_empty(),
        "a replayed seq must not be ingested"
    );

    let rows = ingest.machines(later);
    assert_eq!(rows[0].status, MachineStatus::Unreachable);
    assert!(rows[0].reason.as_deref().unwrap().contains("replayed"));
}

/// (e) agent silent past TTL -> agent Degraded (Unreachable) and its WORKING
/// session renders STALE/UNKNOWN via the ordinary session-TTL path, not
/// stuck WORKING forever.
#[test]
fn silent_agent_past_ttl_leaves_session_stale_not_stuck_working() {
    let dir = tempfile::tempdir().unwrap();
    let secret = b"shared-secret";
    let now = Utc::now();

    let mut publisher = FileTransportPublisher::new(dir.path(), "pc").unwrap();
    let mut ev = session_event("s1", "pc", "work", now);
    ev.ttl_secs = Some(30); // short TTL so the test doesn't need real waits
    let bundle = make_bundle("pc", 1, now, vec![ev]);
    sign_and_publish(&mut publisher, &bundle, secret, "k1");

    let ttl_secs = 60;
    let mut ingest = AgentIngestObserver::new(
        Box::new(FileTransportReceiver::new(dir.path())),
        keyring("k1", secret),
        ttl_secs,
    );
    let events = ingest.poll(now);
    let mut store = StateStore::new();
    store.apply_events(&events, now);
    assert_eq!(
        store.sessions["s1"].displayed_state.value,
        StationState::Working
    );

    // No further bundles ever arrive. Advance well past both the agent TTL
    // and the session's own observation TTL.
    let later = now + Duration::seconds(200);
    let no_new_events = ingest.poll(later); // receiver drains nothing new
    assert!(no_new_events.is_empty());
    store.apply_events(&no_new_events, later); // re-runs apply_ttls or derive_stalls at `later`

    let rows = ingest.machines(later);
    assert_eq!(rows[0].status, MachineStatus::Unreachable);
    assert_eq!(rows[0].reason.as_deref(), Some("agent unreachable"));

    assert_eq!(
        store.sessions["s1"].displayed_state.value,
        StationState::StaleUnknown,
        "a session must expire via the ordinary TTL path once its agent goes silent, never stay WORKING"
    );
}

/// (f) a secret in a label is not present in the transport file on disk.
#[test]
fn secret_in_label_never_reaches_the_transport_file() {
    let dir = tempfile::tempdir().unwrap();
    let secret = b"shared-secret";
    let now = Utc::now();

    let leaking_label = "session for breydenerrett@gmail.com key=sk-abcdefghij1234567890";
    let redacted_label = foundry_core::redact::redact_field(leaking_label);
    let mut ev = session_event("s1", "pc", &redacted_label, now);
    // Simulate what the agent binary does: redact again defensively right
    // before bundling, exactly like `foundry-agent`'s `redact_event`.
    if let Some(l) = &ev.label {
        ev.label = Some(foundry_core::redact::redact_field(l));
    }

    let mut publisher = FileTransportPublisher::new(dir.path(), "pc").unwrap();
    let bundle = make_bundle("pc", 1, now, vec![ev]);
    sign_and_publish(&mut publisher, &bundle, secret, "k1");

    let file_contents = std::fs::read_to_string(dir.path().join("pc.jsonl")).unwrap();
    assert!(
        !file_contents.contains("breydenerrett@gmail.com"),
        "an email must never reach the on-disk transport file: {file_contents}"
    );
    assert!(
        !file_contents.contains("sk-abcdefghij1234567890"),
        "an API-key-shaped secret must never reach the on-disk transport file: {file_contents}"
    );
}

/// (g) F-8: a key_id bound to one agent_id must not authenticate a bundle
/// claiming to be a DIFFERENT agent_id, even though the signature itself is
/// perfectly valid (both agents are handed the same shared secret in this
/// scenario — e.g. a leaked/shared key file).
#[test]
fn key_bound_to_one_agent_id_rejects_a_bundle_claiming_another() {
    let dir = tempfile::tempdir().unwrap();
    let secret = b"shared-secret";
    let now = Utc::now();

    // "pc"'s key gets used to sign a bundle that claims to be "laptop" —
    // exactly what a compromised/misdirected agent process would produce.
    let bundle = make_bundle(
        "laptop",
        1,
        now,
        vec![session_event("s1", "laptop", "work", now)],
    );
    let mut publisher = FileTransportPublisher::new(dir.path(), "laptop").unwrap();
    sign_and_publish(&mut publisher, &bundle, secret, "k-pc");

    let mut bindings = BTreeMap::new();
    bindings.insert("k-pc".to_string(), "pc".to_string());
    let mut ingest = AgentIngestObserver::new(
        Box::new(FileTransportReceiver::new(dir.path())),
        keyring("k-pc", secret),
        120,
    )
    .with_key_bindings(bindings);

    let events = ingest.poll(now);
    assert!(
        events.is_empty(),
        "a key_id bound to a different agent_id must not be ingested"
    );
    let rows = ingest.machines(now);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].agent_id, "laptop");
    assert_eq!(rows[0].status, MachineStatus::Unreachable);
    assert!(rows[0]
        .reason
        .as_deref()
        .unwrap()
        .contains("not authorized"));
}

/// (h) F-8: the SAME key_id, correctly claiming the agent_id it's bound to,
/// still authenticates normally — binding must not break the ordinary case.
#[test]
fn key_bound_to_its_own_agent_id_still_authenticates() {
    let dir = tempfile::tempdir().unwrap();
    let secret = b"shared-secret";
    let now = Utc::now();

    let bundle = make_bundle("pc", 1, now, vec![session_event("s1", "pc", "work", now)]);
    let mut publisher = FileTransportPublisher::new(dir.path(), "pc").unwrap();
    sign_and_publish(&mut publisher, &bundle, secret, "k-pc");

    let mut bindings = BTreeMap::new();
    bindings.insert("k-pc".to_string(), "pc".to_string());
    let mut ingest = AgentIngestObserver::new(
        Box::new(FileTransportReceiver::new(dir.path())),
        keyring("k-pc", secret),
        120,
    )
    .with_key_bindings(bindings);

    let events = ingest.poll(now);
    assert_eq!(events.len(), 1);
    assert_eq!(ingest.machines(now)[0].status, MachineStatus::Reachable);
}

/// (i) F-9: an UNKNOWN key_id (no signature could even be checked) is
/// bucketed under the fixed "unverified" row, never under whatever agent_id
/// the unauthenticated bundle JSON happens to claim.
#[test]
fn unknown_key_id_is_bucketed_as_unverified_not_the_claimed_agent_id() {
    let dir = tempfile::tempdir().unwrap();
    let now = Utc::now();

    // Claims to be the well-known "pc" machine, signed with a key_id main
    // never configured at all.
    let bundle = make_bundle("pc", 1, now, vec![session_event("s1", "pc", "work", now)]);
    let mut publisher = FileTransportPublisher::new(dir.path(), "pc").unwrap();
    sign_and_publish(&mut publisher, &bundle, b"whatever", "nope");

    let mut ingest = AgentIngestObserver::new(
        Box::new(FileTransportReceiver::new(dir.path())),
        keyring("k1", b"secret"),
        120,
    );
    ingest.poll(now);
    let rows = ingest.machines(now);
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].agent_id,
        foundry_core::agents::UNVERIFIED_BUCKET,
        "an unauthenticated bundle must not be able to fabricate a row under its claimed agent_id"
    );
    assert_ne!(
        rows[0].agent_id, "pc",
        "the real 'pc' agent's row must not be created/overwritten by an unverified claim"
    );
}

/// (j) F-10: a rejection reason containing secret-shaped text (an email, a
/// path) is redacted before it is stored/rendered — `ObserverHealth` and
/// `MachineRecord.reason` are both rendered verbatim by `render_floor`.
#[test]
fn rejection_reason_is_redacted_before_it_is_stored() {
    let dir = tempfile::tempdir().unwrap();
    let now = Utc::now();

    // The key_id itself is attacker-controlled (it's read straight off the
    // wire before any verification) — smuggle secret-shaped text into it.
    let leaking_key_id = "leak-breydenerrett@gmail.com";
    let bundle = make_bundle("pc", 1, now, vec![session_event("s1", "pc", "work", now)]);
    let mut publisher = FileTransportPublisher::new(dir.path(), "pc").unwrap();
    sign_and_publish(&mut publisher, &bundle, b"whatever", leaking_key_id);

    let mut ingest = AgentIngestObserver::new(
        Box::new(FileTransportReceiver::new(dir.path())),
        keyring("k1", b"secret"),
        120,
    );
    ingest.poll(now);
    let rows = ingest.machines(now);
    let reason = rows[0].reason.as_deref().unwrap();
    assert!(
        !reason.contains("breydenerrett@gmail.com"),
        "a rejection reason must be redacted before it is stored/rendered: {reason}"
    );
}

/// (k) F-11: `agent_id -> last accepted seq` survives a restart via
/// `StateStore::agent_seq_watermarks`, so a fresh `AgentIngestObserver`
/// seeded from it refuses a replay of the last-ever-accepted seq — not just
/// bare `seq == 0` (that narrower guarantee is `replayed_seq_zero_bundle_
/// must_not_resurrect_a_dead_machine` in `tests/opus_review.rs`).
#[test]
fn seq_watermark_survives_restart_and_still_rejects_a_replay() {
    let dir = tempfile::tempdir().unwrap();
    let secret = b"shared-secret";
    let now = Utc::now();

    let mut publisher = FileTransportPublisher::new(dir.path(), "pc").unwrap();
    let bundle = make_bundle("pc", 7, now, vec![session_event("s1", "pc", "work", now)]);
    sign_and_publish(&mut publisher, &bundle, secret, "k1");

    let mut store = StateStore::new();
    {
        let mut ingest = AgentIngestObserver::new(
            Box::new(FileTransportReceiver::new(dir.path())),
            keyring("k1", secret),
            120,
        );
        let events = ingest.poll(now);
        assert_eq!(events.len(), 1);
        store.set_agent_seq_watermarks(ingest.seq_watermarks());
        assert_eq!(store.agent_seq_watermarks.get("pc"), Some(&7));
        // `ingest` (and its in-memory replay guard) is dropped here — a
        // brand new process would start with nothing but `store`.
    }

    // A byte-identical envelope (seq 7 again) is replayed after "restart".
    publisher
        .publish({
            let bundle_json = serde_json::to_string(&bundle).unwrap();
            let sig_hex = sign::sign_hex(secret, bundle_json.as_bytes());
            SignedBundle {
                bundle_json,
                sig_hex,
                key_id: "k1".to_string(),
            }
        })
        .unwrap();

    let mut fresh_ingest = AgentIngestObserver::new(
        Box::new(FileTransportReceiver::new(dir.path())),
        keyring("k1", secret),
        120,
    );
    fresh_ingest.restore_seq_watermarks(&store.agent_seq_watermarks);
    let later = now + Duration::seconds(5);
    let events = fresh_ingest.poll(later);
    assert!(
        events.is_empty(),
        "a restart-seeded watermark must still reject a replay of the last-accepted seq"
    );
    let rows = fresh_ingest.machines(later);
    assert_eq!(rows[0].status, MachineStatus::Unreachable);
    assert!(rows[0].reason.as_deref().unwrap().contains("replayed"));
}
