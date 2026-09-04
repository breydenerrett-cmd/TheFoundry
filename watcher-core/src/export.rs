//! L-01: the JSON wire shape the Pixi app polls. `FloorState` mirrors
//! `app/src/state.ts` field-for-field — that TypeScript file is the
//! contract; this module must never drift from it silently.
//!
//! Every derived number here comes from the SAME `StateStore` methods
//! `render.rs`'s marquee uses (`session_state_counts`, `overdue_routines_count`,
//! `next_routine`, `pipeline_verified`) — the live text screen and this JSON
//! export can never quietly disagree about what the floor looks like.
//!
//! Every free-text string (labels, error text, routine names) passes through
//! `redact::redact_field` again here, even though observers already redact
//! at ingestion — belt and suspenders for the one artifact this process
//! hands to an untrusted browser tab.

use crate::bay::{BayMap, UNRESOLVED};
use crate::eventlog::EventLog;
use crate::redact::redact_field;
use crate::reducer::{MachineStatus, StateStore};
use crate::schema::{Event, Fidelity, StationState};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::BTreeMap;

/// §16's REMOTE ESTATE staleness cutoff — must match `render.rs`'s
/// `REMOTE_STALE_SECS` exactly, or the text screen and the JSON export would
/// disagree about when the remote estate reads DEGRADED.
pub const REMOTE_STALE_SECS: i64 = 30 * 60;
/// Same canary freshness window `main.rs`/`render.rs` use for
/// `pipeline_verified`.
pub const MAX_CANARY_AGE_SECS: i64 = 300;
/// Cap on how many tape rows a single export carries.
pub const MAX_TAPE_EVENTS: usize = 200;

#[derive(Debug, Clone, Serialize)]
pub struct SessionOut {
    pub id: String,
    pub bay: String,
    pub state: String,
    pub fidelity: Fidelity,
    pub model: Option<String>,
    pub model_current: Option<String>,
    pub elapsed_secs: i64,
    pub label: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub restored: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_kind: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RoutineOut {
    pub id: String,
    pub name: String,
    pub bay: String,
    pub enabled: bool,
    pub overdue: bool,
    pub next_run_at: Option<DateTime<Utc>>,
    pub stale: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub restored: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckOut {
    pub id: String,
    pub label: String,
    pub bay: String,
    /// The current `CheckRecord` shape carries no observed ok/fail boolean —
    /// always `None` (unknown) rather than fabricating a confident zero.
    pub ok: Option<bool>,
    pub last_event_ts: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub restored: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ObserverOut {
    pub name: String,
    pub status: crate::health::ObserverStatus,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub consecutive_failures: u32,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MachineOut {
    pub id: String,
    pub name: String,
    pub reachable: bool,
    pub last_seen_secs: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TapeEventOut {
    pub ts: DateTime<Utc>,
    pub source: String,
    pub kind: String,
    pub entity: String,
    pub state: String,
    pub fidelity: Fidelity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteEstate {
    Live,
    Degraded,
    NotRunning,
}

#[derive(Debug, Clone, Serialize)]
pub struct PipelineOut {
    pub verified: bool,
    pub remote_estate: RemoteEstate,
    pub last_sync_age_secs: Option<i64>,
    pub last_output_age_secs: Option<i64>,
    pub next_routine: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShelfSlotOut {
    pub kind: &'static str,
    pub count: Option<u32>,
    pub fidelity: Fidelity,
}

#[derive(Debug, Clone, Serialize)]
pub struct FloorState {
    pub generated_at: DateTime<Utc>,
    pub sessions: Vec<SessionOut>,
    pub routines: Vec<RoutineOut>,
    pub checks: Vec<CheckOut>,
    pub observers: Vec<ObserverOut>,
    pub pipeline: PipelineOut,
    pub output_shelf: BTreeMap<String, Vec<ShelfSlotOut>>,
    pub machines: Vec<MachineOut>,
    pub tape: Vec<TapeEventOut>,
}

fn station_state_str(s: StationState) -> &'static str {
    match s {
        StationState::Working => "working",
        StationState::Thinking => "thinking",
        StationState::Specialist => "specialist",
        StationState::WaitingOnAgent => "waiting_on_agent",
        StationState::WaitingOnSystem => "waiting_on_system",
        StationState::Blocked => "blocked",
        StationState::BreyRequired => "brey_required",
        StationState::Failed => "failed",
        StationState::Hung => "hung",
        StationState::Idle => "idle",
        StationState::Completed => "completed",
        StationState::StaleUnknown => "stale_unknown",
    }
}

/// The six real §9 bays plus UNRESOLVED, matching `bay::BayMap::BAYS` +
/// `bay::UNRESOLVED` — the fixed set `output_shelf` always carries a row for.
const ARTIFACT_KINDS: &[&str] = &["commit", "test", "deploy", "capture", "completed"];

fn build_output_shelf() -> BTreeMap<String, Vec<ShelfSlotOut>> {
    let mut shelf = BTreeMap::new();
    for bay in crate::bay::BAYS.iter().chain(std::iter::once(&UNRESOLVED)) {
        // S-01 truth rule: the current watcher-core does not yet instrument
        // per-bay artifact velocity (§5a is only a first-cut rollup via
        // `last_output_at`) — every slot is honestly "no instrumentation"
        // (`count: None`, `fidelity: Unknown`), never a fabricated zero.
        let slots = ARTIFACT_KINDS
            .iter()
            .map(|k| ShelfSlotOut {
                kind: k,
                count: None,
                fidelity: Fidelity::Unknown,
            })
            .collect();
        shelf.insert((*bay).to_string(), slots);
    }
    shelf
}

fn remote_estate(store: &StateStore, now: DateTime<Utc>) -> RemoteEstate {
    match store.observer_health.get("remote_claude") {
        Some(h) => match h.last_sync_age_secs(now) {
            Some(age) if age <= REMOTE_STALE_SECS => RemoteEstate::Live,
            _ => RemoteEstate::Degraded,
        },
        None => RemoteEstate::NotRunning,
    }
}

/// Builds the exported `FloorState` from `store` at `now`. `tape` is the
/// event-log ring buffer (`EventLog::ring_snapshot()`); only the last
/// `MAX_TAPE_EVENTS` are kept, redacted to shape-only fields.
pub fn build_floor_state(
    store: &StateStore,
    now: DateTime<Utc>,
    bay_map: &BayMap,
    tape: &[Event],
) -> FloorState {
    let mut sessions = Vec::new();
    for rec in store.sessions.values() {
        let (state, fidelity) = if rec.gone {
            let faded_secs = rec
                .gone_at
                .map(|t| (now - t).num_seconds())
                .unwrap_or(i64::MAX);
            if faded_secs > crate::reducer::GONE_FADE_SECS {
                // Past the fade window — folded off the floor entirely,
                // exactly like `render.rs`'s ended-session handling.
                continue;
            }
            ("fading_ended".to_string(), Fidelity::Unknown)
        } else {
            (
                station_state_str(rec.displayed_state.value).to_string(),
                rec.displayed_state.fidelity,
            )
        };
        let bay = bay_map.resolve(rec.repo_hint.as_deref(), &[]).bay;
        let elapsed_secs = (now - rec.last_activity_at).num_seconds().max(0);
        sessions.push(SessionOut {
            id: redact_field(&rec.id),
            bay,
            state,
            fidelity,
            model: rec.model.as_ref().map(|f| redact_field(&f.value)),
            model_current: rec.model_current.as_ref().map(|f| redact_field(&f.value)),
            elapsed_secs,
            label: rec
                .label
                .as_ref()
                .map(|f| redact_field(&f.value))
                .unwrap_or_else(|| "(no task summary)".to_string()),
            restored: rec.restored,
            session_kind: rec.session_kind.as_ref().map(|f| redact_field(&f.value)),
        });
    }

    let routines = store
        .routines
        .values()
        .map(|r| RoutineOut {
            id: redact_field(&r.id),
            name: redact_field(&r.name),
            // RoutineRecord carries no repo/bay hint today — honestly
            // UNRESOLVED rather than guessed.
            bay: UNRESOLVED.to_string(),
            enabled: r.enabled,
            overdue: r.overdue,
            next_run_at: r.next_run_at,
            stale: r.stale,
            restored: r.restored,
        })
        .collect();

    let checks = store
        .checks
        .values()
        .map(|c| CheckOut {
            id: redact_field(&c.id),
            label: redact_field(&c.label.value),
            bay: bay_map.resolve(c.repo_hint.as_deref(), &[]).bay,
            ok: None,
            last_event_ts: c.last_event_ts,
            restored: c.restored,
        })
        .collect();

    let observers = store
        .observer_health
        .values()
        .map(|h| ObserverOut {
            name: redact_field(&h.name),
            status: h.status,
            last_success_at: h.last_success_at,
            last_error: h.last_error.as_ref().map(|e| redact_field(e)),
            consecutive_failures: h.consecutive_failures,
            capabilities: h.capabilities.0.iter().map(|c| redact_field(c)).collect(),
        })
        .collect();

    let machines = store
        .machines
        .values()
        .map(|m| MachineOut {
            id: redact_field(&m.agent_id),
            name: redact_field(&m.agent_id),
            reachable: m.status == MachineStatus::Reachable,
            last_seen_secs: m.last_heard_at.map(|t| (now - t).num_seconds().max(0)),
        })
        .collect();

    let tape_out = tape
        .iter()
        .rev()
        .take(MAX_TAPE_EVENTS)
        .rev()
        .map(|ev| TapeEventOut {
            ts: ev.ts,
            source: redact_field(&ev.source),
            kind: serde_json::to_value(ev.kind)
                .ok()
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_default(),
            entity: redact_field(&ev.entity.id),
            state: ev.state.map(station_state_str).unwrap_or("n/a").to_string(),
            fidelity: ev.fidelity,
        })
        .collect();

    let pipeline = PipelineOut {
        verified: store.pipeline_verified(now, MAX_CANARY_AGE_SECS),
        remote_estate: remote_estate(store, now),
        last_sync_age_secs: store.last_sync_age_secs(now),
        last_output_age_secs: store
            .last_output_at()
            .map(|t| (now - t).num_seconds().max(0)),
        next_routine: store
            .next_routine()
            .map(|(t, name)| format!("{} ({})", redact_field(name), t.format("%H:%M UTC"))),
    };

    FloorState {
        generated_at: now,
        sessions,
        routines,
        checks,
        observers,
        pipeline,
        output_shelf: build_output_shelf(),
        machines,
        tape: tape_out,
    }
}

/// Fetches the event-log ring buffer through the `&mut EventLog` API — a
/// tiny wrapper so `main.rs` doesn't need to know `ring_snapshot` exists
/// separately from `build_floor_state`.
pub fn tape_from_log(log: &EventLog) -> Vec<Event> {
    log.ring_snapshot()
}
