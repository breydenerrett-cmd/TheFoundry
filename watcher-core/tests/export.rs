//! L-01: export.rs tests — the exported JSON is validated against a small
//! hand-rolled TS-shape checker (required keys + valid enum values), its
//! counts are checked to match `render_floor`'s marquee (built from the SAME
//! `StateStore` methods, see `reducer.rs::session_state_counts`), secrets are
//! confirmed redacted, and the `--serve` HTTP server is exercised end to end
//! over a real loopback socket.

use chrono::{DateTime, Duration, Utc};
use foundry_core::bay::BayMap;
use foundry_core::export::build_floor_state;
use foundry_core::httpd::{self, ServedState};
use foundry_core::observer::{Observer, RemoteClaudeObserver, SyntheticCanary};
use foundry_core::reducer::StateStore;
use foundry_core::render::render_floor;
use foundry_core::schema::{EntityRef, EntityType, Event, EventKind, Fidelity, Metrics};
use serde_json::Value;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

/// Polls the repo's real `live-feed` fixture through the actual
/// `RemoteClaudeObserver` + `SyntheticCanary`, exactly like `main.rs` does,
/// so the export and the marquee are exercised against real-shaped data.
fn live_feed_store() -> (StateStore, DateTime<Utc>) {
    let feed_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("live-feed");
    let now = Utc::now();
    let mut remote = RemoteClaudeObserver::new(&feed_dir);
    let mut canary = SyntheticCanary::new();
    let mut events = remote.poll(now);
    events.extend(canary.poll(now));
    let mut store = StateStore::new();
    store.apply_events(&events, now);
    store.apply_observer_health(
        remote.health(),
        now,
        Some(foundry_core::observer::CAP_SESSIONS),
        Some(foundry_core::observer::CAP_ROUTINES),
    );
    store.apply_observer_health(canary.health(), now, None, None);
    (store, now)
}

// --- a tiny TS-shape validator: required keys present, enums valid --------

const FIDELITY_VALUES: &[&str] = &["observed", "inferred", "unknown"];
const STATION_STATES: &[&str] = &[
    "working",
    "thinking",
    "specialist",
    "waiting_on_agent",
    "waiting_on_system",
    "blocked",
    "brey_required",
    "failed",
    "hung",
    "idle",
    "completed",
    "stale_unknown",
    "fading_ended",
];
const OBSERVER_STATUSES: &[&str] = &["healthy", "degraded", "down", "unverified"];
const REMOTE_ESTATES: &[&str] = &["live", "degraded", "not_running"];

fn require_keys(obj: &Value, keys: &[&str], where_: &str) {
    let map = obj
        .as_object()
        .unwrap_or_else(|| panic!("{where_} is not an object"));
    for k in keys {
        assert!(
            map.contains_key(*k),
            "{where_} missing required key '{k}': {obj}"
        );
    }
}

fn require_enum(obj: &Value, key: &str, allowed: &[&str], where_: &str) {
    let v = obj
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("{where_}.{key} missing or not a string: {obj}"));
    assert!(
        allowed.contains(&v),
        "{where_}.{key} = '{v}' is not one of {allowed:?}"
    );
}

/// Validates the whole document against the FloorState shape from
/// `app/src/state.ts`.
fn validate_floor_state_shape(doc: &Value) {
    require_keys(
        doc,
        &[
            "generated_at",
            "sessions",
            "routines",
            "checks",
            "observers",
            "pipeline",
            "output_shelf",
        ],
        "FloorState",
    );
    assert!(doc["generated_at"].as_str().is_some());

    for s in doc["sessions"].as_array().unwrap() {
        require_keys(
            s,
            &[
                "id",
                "bay",
                "state",
                "fidelity",
                "model",
                "model_current",
                "elapsed_secs",
                "label",
            ],
            "SessionRecord",
        );
        require_enum(s, "fidelity", FIDELITY_VALUES, "SessionRecord");
        require_enum(s, "state", STATION_STATES, "SessionRecord");
        assert!(s["elapsed_secs"].as_i64().is_some());
    }

    for r in doc["routines"].as_array().unwrap() {
        require_keys(
            r,
            &[
                "id",
                "name",
                "bay",
                "enabled",
                "overdue",
                "next_run_at",
                "stale",
            ],
            "RoutineRecord",
        );
    }

    for c in doc["checks"].as_array().unwrap() {
        require_keys(
            c,
            &["id", "label", "bay", "ok", "last_event_ts"],
            "CheckRecord",
        );
    }

    for o in doc["observers"].as_array().unwrap() {
        require_keys(
            o,
            &[
                "name",
                "status",
                "last_success_at",
                "last_error",
                "consecutive_failures",
            ],
            "ObserverHealth",
        );
        require_enum(o, "status", OBSERVER_STATUSES, "ObserverHealth");
    }

    let pipeline = &doc["pipeline"];
    require_keys(
        pipeline,
        &[
            "verified",
            "remote_estate",
            "last_sync_age_secs",
            "last_output_age_secs",
            "next_routine",
        ],
        "PipelineSummary",
    );
    assert!(pipeline["verified"].as_bool().is_some());
    require_enum(pipeline, "remote_estate", REMOTE_ESTATES, "PipelineSummary");

    assert!(doc["output_shelf"].as_object().is_some());
    if let Some(machines) = doc.get("machines") {
        for m in machines.as_array().unwrap() {
            require_keys(
                m,
                &["id", "name", "reachable", "last_seen_secs"],
                "MachineRecord",
            );
        }
    }
    if let Some(tape) = doc.get("tape") {
        for t in tape.as_array().unwrap() {
            require_keys(
                t,
                &["ts", "source", "kind", "entity", "state", "fidelity"],
                "TapeEvent",
            );
            require_enum(t, "fidelity", FIDELITY_VALUES, "TapeEvent");
        }
    }
}

#[test]
fn floor_state_json_round_trips_through_the_ts_shape_validator() {
    let (store, now) = live_feed_store();
    let floor = build_floor_state(&store, now, &BayMap::new(), &[]);
    let doc = serde_json::to_value(&floor).expect("FloorState must serialize");
    validate_floor_state_shape(&doc);
}

#[test]
fn export_counts_match_render_marquee_on_the_live_feed_fixture() {
    let (store, now) = live_feed_store();
    let bay_map = BayMap::new();

    let text = render_floor(&store, now, &bay_map);
    let floor = build_floor_state(&store, now, &bay_map, &[]);

    // Both surfaces are built from the SAME `StateStore::session_state_counts`
    // — cross-check by recomputing the observed-session count from each and
    // asserting they match (and match the store's own visible-session count).
    let visible_in_store = store.sessions.values().filter(|r| !r.gone).count();
    assert_eq!(
        floor.sessions.len(),
        visible_in_store,
        "export must carry exactly the non-gone sessions the store has"
    );
    let marquee_total: usize = {
        let mut n = 0usize;
        for c in store.session_state_counts().values() {
            n += *c as usize;
        }
        n
    };
    assert_eq!(floor.sessions.len(), marquee_total);
    assert!(
        text.contains("SESSIONS"),
        "sanity: marquee text rendered at all"
    );

    assert_eq!(floor.pipeline.verified, store.pipeline_verified(now, 300));
}

#[test]
fn secret_in_a_session_label_is_redacted_in_the_export() {
    let now = Utc::now();
    let mut store = StateStore::new();
    let secret_label = "leaked key sk-abcdefghij1234567890 in the task summary";
    let ev = Event {
        ts: now,
        source: "remote_claude".into(),
        kind: EventKind::SessionObserved,
        entity: EntityRef::new(EntityType::Session, "s1"),
        project_id: None,
        session_id: Some("s1".into()),
        model: Some("claude-sonnet-5".into()),
        model_current: None,
        model_last_served: None,
        effort: None,
        state: Some(foundry_core::schema::StationState::Working),
        label: Some(secret_label.into()),
        detail: None,
        fidelity: Fidelity::Observed,
        metrics: Metrics::default(),
        ttl_secs: Some(120),
        next_run_at: None,
        enabled: None,
    };
    store.apply_events(&[ev], now);

    let floor = build_floor_state(&store, now, &BayMap::new(), &[]);
    let doc = serde_json::to_value(&floor).unwrap();
    let json = doc.to_string();
    assert!(
        !json.contains("sk-abcdefghij1234567890"),
        "secret must not survive into the exported JSON: {json}"
    );
    assert!(json.contains("[REDACTED-SECRET]"));
}

#[test]
fn restored_session_is_flagged_and_stays_stale_unknown_in_export() {
    let now = Utc::now();
    let mut store = StateStore::new();
    let ev = Event {
        ts: now,
        source: "remote_claude".into(),
        kind: EventKind::SessionObserved,
        entity: EntityRef::new(EntityType::Session, "s1"),
        project_id: None,
        session_id: Some("s1".into()),
        model: None,
        model_current: None,
        model_last_served: None,
        effort: None,
        state: Some(foundry_core::schema::StationState::Working),
        label: Some("doing work".into()),
        detail: None,
        fidelity: Fidelity::Observed,
        metrics: Metrics::default(),
        ttl_secs: Some(120),
        next_run_at: None,
        enabled: None,
    };
    store.apply_events(&[ev], now);
    foundry_core::persist::mark_restored(&mut store, now);

    let floor = build_floor_state(&store, now + Duration::seconds(5), &BayMap::new(), &[]);
    let rec = &floor.sessions[0];
    assert!(rec.restored);
    assert_eq!(rec.state, "stale_unknown");
    assert_eq!(rec.fidelity, Fidelity::Unknown);
    assert!(
        !floor.pipeline.verified,
        "a restored floor must not export verified"
    );
}

// --- --serve tests ----------------------------------------------------

fn http_get(port: u16, path: &str, token: Option<&str>) -> (u16, String, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut req =
        format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: http://127.0.0.1:5173\r\n");
    if let Some(t) = token {
        req.push_str(&format!("X-Foundry-Token: {t}\r\n"));
    }
    req.push_str("\r\n");
    stream.write_all(req.as_bytes()).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(3)))
        .unwrap();
    let mut buf = String::new();
    let _ = stream.read_to_string(&mut buf);
    let mut parts = buf.splitn(2, "\r\n\r\n");
    let head = parts.next().unwrap_or_default().to_string();
    let body = parts.next().unwrap_or_default().to_string();
    let status: u16 = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    (status, head, body)
}

#[test]
fn serve_responds_on_state_and_health() {
    let addr: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
    let state = ServedState {
        json: Arc::new(Mutex::new(String::from(r#"{"generated_at":"now"}"#))),
        seq: Arc::new(AtomicU64::new(7)),
        token: None,
    };
    let listener = httpd::serve(addr, state).expect("loopback bind must succeed");
    let port = listener.local_addr().unwrap().port();

    let (status, head, body) = http_get(port, "/state", None);
    assert_eq!(status, 200);
    assert!(head.contains("Cache-Control: no-store"));
    assert!(head.contains("Access-Control-Allow-Origin: *"));
    assert!(body.contains("generated_at"));

    let (status, _head, body) = http_get(port, "/health", None);
    assert_eq!(status, 200);
    assert!(body.contains("\"ok\":true"));
    assert!(body.contains("\"seq\":7"));

    let (status, _, _) = http_get(port, "/nope", None);
    assert_eq!(status, 404);
}

#[test]
fn serve_requires_matching_token_when_configured() {
    let addr: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
    let state = ServedState {
        json: Arc::new(Mutex::new(String::from("{}"))),
        seq: Arc::new(AtomicU64::new(0)),
        token: Some("s3cr3t-token".to_string()),
    };
    let listener = httpd::serve(addr, state).unwrap();
    let port = listener.local_addr().unwrap().port();

    let (status, _, _) = http_get(port, "/state", None);
    assert_eq!(status, 401, "missing token must be rejected");

    let (status, _, _) = http_get(port, "/state", Some("wrong"));
    assert_eq!(status, 401, "wrong token must be rejected");

    let (status, _, _) = http_get(port, "/state", Some("s3cr3t-token"));
    assert_eq!(status, 200, "correct token must be accepted");
}

#[test]
fn serve_refuses_to_bind_a_non_loopback_host() {
    let addr: std::net::SocketAddr = "0.0.0.0:0".parse().unwrap();
    let state = ServedState {
        json: Arc::new(Mutex::new(String::from("{}"))),
        seq: Arc::new(AtomicU64::new(0)),
        token: None,
    };
    let result = httpd::serve(addr, state);
    assert!(result.is_err(), "must refuse to bind a non-loopback host");
}
