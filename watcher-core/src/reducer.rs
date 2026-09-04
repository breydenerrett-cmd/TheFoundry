//! State reducer (§8): folds the normalized event stream into a state store.
//! Owns the two rules that keep the floor honest:
//!   1. absence is UNKNOWN, never healthy (an observer going Down degrades
//!      every record it sources, it does not freeze them at their last-good
//!      value forever);
//!   2. STALL_SUSPECTED / STALL_CONFIRMED derivation lives here, in one place,
//!      not scattered across observers.

use crate::health::{ObserverHealth, ObserverStatus};
use crate::schema::{Event, EventKind, Fidelity, StationState};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The value/observed_at/fidelity triple (§8) — every displayed fact carries
/// this so the UI can honestly render staleness and inference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Field<T> {
    pub value: T,
    pub observed_at: DateTime<Utc>,
    pub fidelity: Fidelity,
}

impl<T> Field<T> {
    pub fn new(value: T, observed_at: DateTime<Utc>, fidelity: Fidelity) -> Self {
        Self { value, observed_at, fidelity }
    }
}

/// §6's default thresholds. Coarse-telemetry sessions (remote, snapshot-based)
/// only get the "between turns" threshold — we cannot see mid-tool activity
/// for them, which is itself a fidelity limitation worth being honest about.
pub const STALL_CONFIRM_SECS: i64 = 25 * 60;
pub const STALL_WARN_FRACTION: f64 = 0.6;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub source: String,
    /// Observed state as reported by the observer — before stall derivation.
    pub observed_state: Field<StationState>,
    /// Displayed state — observed_state, unless the reducer has overridden it
    /// (stall derivation, or the owning observer going Down).
    pub displayed_state: Field<StationState>,
    pub model: Option<Field<String>>,
    pub model_current: Option<Field<String>>,
    pub model_last_served: Option<Field<String>>,
    pub label: Option<Field<String>>,
    pub session_kind: Option<Field<String>>,
    /// Last time we successfully polled and this session appeared in the
    /// snapshot at all (used for absence bookkeeping — NOT for staleness).
    pub last_polled_at: DateTime<Utc>,
    /// The session's own last-activity timestamp, as reported by the
    /// observer (derived from its `updated_at`, never from our poll clock).
    /// This is what elapsed-time display and stall derivation must use — a
    /// session polled every 10s for an hour with no real activity must still
    /// read as an hour stale, not as "just seen".
    pub last_activity_at: DateTime<Utc>,
    pub stall_warning: bool,
    pub gone: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutineRecord {
    pub id: String,
    pub name: String,
    pub bound_session_id: Option<String>,
    pub overdue: bool,
    pub enabled: bool,
    pub next_run_at: Option<DateTime<Utc>>,
    pub prompt_redacted: Option<String>,
    pub last_seen_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PipelineHealth {
    pub last_canary_at: Option<DateTime<Utc>>,
    pub last_any_event_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StateStore {
    pub sessions: BTreeMap<String, SessionRecord>,
    pub routines: BTreeMap<String, RoutineRecord>,
    pub observer_health: BTreeMap<String, ObserverHealth>,
    pub pipeline: PipelineHealth,
}

impl StateStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply_events(&mut self, events: &[Event], now: DateTime<Utc>) {
        for ev in events {
            self.pipeline.last_any_event_at = Some(now);
            match ev.kind {
                EventKind::Heartbeat => {
                    self.pipeline.last_canary_at = Some(ev.ts);
                }
                EventKind::SessionObserved => self.apply_session_observed(ev, now),
                EventKind::SessionGone => {
                    if let Some(rec) = self.sessions.get_mut(&ev.entity.id) {
                        rec.gone = true;
                        rec.displayed_state = Field::new(StationState::StaleUnknown, now, Fidelity::Observed);
                    }
                }
                EventKind::RoutineScheduled | EventKind::RoutineOverdue => {
                    self.apply_routine(ev, now, ev.kind == EventKind::RoutineOverdue);
                }
                _ => {}
            }
        }
        self.derive_stalls(now);
    }

    fn apply_session_observed(&mut self, ev: &Event, now: DateTime<Utc>) {
        let Some(state) = ev.state else { return };
        // Derive the session's real last-activity time from the observer's
        // elapsed_ms metric (itself computed from the provider's updated_at),
        // NOT from `now` — `now` is only when WE polled, which says nothing
        // about whether the session actually did anything since last time.
        let activity_at = ev
            .metrics
            .elapsed_ms
            .map(|ms| now - chrono::Duration::milliseconds(ms))
            .unwrap_or(now);

        let entry = self.sessions.entry(ev.entity.id.clone()).or_insert_with(|| SessionRecord {
            id: ev.entity.id.clone(),
            source: ev.source.clone(),
            observed_state: Field::new(state, now, ev.fidelity),
            displayed_state: Field::new(state, now, ev.fidelity),
            model: None,
            model_current: None,
            model_last_served: None,
            label: None,
            session_kind: None,
            last_polled_at: now,
            last_activity_at: activity_at,
            stall_warning: false,
            gone: false,
        });
        entry.observed_state = Field::new(state, now, ev.fidelity);
        entry.displayed_state = Field::new(state, now, ev.fidelity);
        entry.source = ev.source.clone();
        entry.last_polled_at = now;
        entry.last_activity_at = activity_at;
        entry.gone = false;
        if let Some(m) = &ev.model {
            entry.model = Some(Field::new(m.clone(), now, ev.fidelity));
        }
        if let Some(m) = &ev.model_current {
            entry.model_current = Some(Field::new(m.clone(), now, ev.fidelity));
        }
        if let Some(m) = &ev.model_last_served {
            entry.model_last_served = Some(Field::new(m.clone(), now, ev.fidelity));
        }
        if let Some(l) = &ev.label {
            entry.label = Some(Field::new(l.clone(), now, ev.fidelity));
        }
        if let Some(k) = &ev.detail {
            entry.session_kind = Some(Field::new(k.clone(), now, ev.fidelity));
        }
    }

    fn apply_routine(&mut self, ev: &Event, now: DateTime<Utc>, overdue: bool) {
        let entry = self.routines.entry(ev.entity.id.clone()).or_insert_with(|| RoutineRecord {
            id: ev.entity.id.clone(),
            name: ev.label.clone().unwrap_or_default(),
            bound_session_id: ev.session_id.clone(),
            overdue,
            enabled: ev.enabled.unwrap_or(true),
            next_run_at: ev.next_run_at,
            prompt_redacted: ev.detail.clone(),
            last_seen_at: now,
        });
        entry.name = ev.label.clone().unwrap_or_else(|| entry.name.clone());
        entry.bound_session_id = ev.session_id.clone().or(entry.bound_session_id.clone());
        entry.overdue = overdue;
        entry.enabled = ev.enabled.unwrap_or(entry.enabled);
        entry.next_run_at = ev.next_run_at.or(entry.next_run_at);
        entry.prompt_redacted = ev.detail.clone().or(entry.prompt_redacted.clone());
        entry.last_seen_at = now;
    }

    /// Rule 2: STALL derivation. Only sessions the observer reports as
    /// Working can go Hung — everything else is already an intentional
    /// "stopped" state and hanging is not a meaningful concept for it.
    fn derive_stalls(&mut self, now: DateTime<Utc>) {
        for rec in self.sessions.values_mut() {
            if rec.gone {
                continue;
            }
            if rec.observed_state.value != StationState::Working {
                rec.stall_warning = false;
                continue;
            }
            let elapsed = (now - rec.last_activity_at).num_seconds();
            let warn_at = (STALL_CONFIRM_SECS as f64 * STALL_WARN_FRACTION) as i64;
            rec.stall_warning = elapsed >= warn_at;
            if elapsed >= STALL_CONFIRM_SECS {
                rec.displayed_state = Field::new(StationState::Hung, now, Fidelity::Inferred);
            }
        }
    }

    /// Rule 1: absence is UNKNOWN. Call once per poll cycle per observer,
    /// after `observer.poll()`, passing its current health. ANY non-Healthy
    /// status (Degraded on the very first failed poll, or Down after
    /// repeated ones) degrades every record it sources to StaleUnknown — it
    /// does NOT leave them frozen at their last-good value, which would
    /// silently look healthy forever. `ObserverStatus::Down` only escalates
    /// the *observer's own* health badge (and the marquee's alarm level);
    /// it is not the threshold for whether the data it sourced can be
    /// trusted — a single failed poll already means "we don't know".
    pub fn apply_observer_health(&mut self, health: &ObserverHealth, now: DateTime<Utc>) {
        let should_degrade = !matches!(health.status, ObserverStatus::Healthy);
        self.observer_health.insert(health.name.clone(), health.clone());
        if should_degrade {
            for rec in self.sessions.values_mut() {
                if rec.source == health.name {
                    rec.displayed_state = Field::new(StationState::StaleUnknown, now, Fidelity::Unknown);
                }
            }
        }
    }

    /// §16 "false everything-healthy" defense: if no canary heartbeat has
    /// landed recently, the floor must NOT be trusted, no matter how good the
    /// last snapshot looked.
    pub fn pipeline_verified(&self, now: DateTime<Utc>, max_canary_age_secs: i64) -> bool {
        match self.pipeline.last_canary_at {
            Some(t) => (now - t).num_seconds() <= max_canary_age_secs,
            None => false,
        }
    }

    pub fn last_sync_age_secs(&self, now: DateTime<Utc>) -> Option<i64> {
        self.pipeline.last_any_event_at.map(|t| (now - t).num_seconds().max(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{EntityRef, EntityType, Metrics};

    fn session_event(id: &str, state: StationState, fidelity: Fidelity, ts: DateTime<Utc>) -> Event {
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
            label: Some("doing work".into()),
            detail: Some("anthropic_cloud".into()),
            fidelity,
            metrics: Metrics::default(),
            ttl_secs: Some(120),
            next_run_at: None,
            enabled: None,
        }
    }

    #[test]
    fn brey_required_is_never_downgraded_to_generic_waiting() {
        let mut store = StateStore::new();
        let now = Utc::now();
        store.apply_events(&[session_event("s1", StationState::BreyRequired, Fidelity::Observed, now)], now);
        let rec = &store.sessions["s1"];
        assert_eq!(rec.displayed_state.value, StationState::BreyRequired);
    }

    #[test]
    fn working_session_goes_hung_after_threshold_with_inferred_fidelity() {
        let mut store = StateStore::new();
        let t0 = Utc::now();
        store.apply_events(&[session_event("s1", StationState::Working, Fidelity::Observed, t0)], t0);
        assert_eq!(store.sessions["s1"].displayed_state.value, StationState::Working);

        let later = t0 + chrono::Duration::seconds(STALL_CONFIRM_SECS + 5);
        store.derive_stalls(later);
        let rec = &store.sessions["s1"];
        assert_eq!(rec.displayed_state.value, StationState::Hung);
        assert_eq!(rec.displayed_state.fidelity, Fidelity::Inferred);
    }

    #[test]
    fn stall_warning_flips_before_full_confirmation() {
        let mut store = StateStore::new();
        let t0 = Utc::now();
        store.apply_events(&[session_event("s1", StationState::Working, Fidelity::Observed, t0)], t0);
        let warn_time = t0 + chrono::Duration::seconds((STALL_CONFIRM_SECS as f64 * 0.7) as i64);
        store.derive_stalls(warn_time);
        let rec = &store.sessions["s1"];
        assert!(rec.stall_warning);
        assert_eq!(rec.displayed_state.value, StationState::Working, "still Working, only warning, below full threshold");
    }

    #[test]
    fn non_working_states_never_derive_hung() {
        let mut store = StateStore::new();
        let t0 = Utc::now();
        store.apply_events(&[session_event("s1", StationState::Idle, Fidelity::Observed, t0)], t0);
        let later = t0 + chrono::Duration::seconds(STALL_CONFIRM_SECS * 10);
        store.derive_stalls(later);
        assert_eq!(store.sessions["s1"].displayed_state.value, StationState::Idle);
    }

    #[test]
    fn observer_going_down_degrades_its_sessions_to_stale_not_frozen_healthy() {
        let mut store = StateStore::new();
        let t0 = Utc::now();
        store.apply_events(&[session_event("s1", StationState::Working, Fidelity::Observed, t0)], t0);
        assert_eq!(store.sessions["s1"].displayed_state.value, StationState::Working);

        let mut health = ObserverHealth::new("remote_claude");
        health.record_failure("e1");
        health.record_failure("e2");
        health.record_failure("e3"); // -> Down
        assert_eq!(health.status, ObserverStatus::Down);

        store.apply_observer_health(&health, t0 + chrono::Duration::seconds(30));
        assert_eq!(store.sessions["s1"].displayed_state.value, StationState::StaleUnknown);
        assert_eq!(store.sessions["s1"].displayed_state.fidelity, Fidelity::Unknown);
    }

    #[test]
    fn pipeline_not_verified_without_recent_canary() {
        let store = StateStore::new();
        assert!(!store.pipeline_verified(Utc::now(), 300));
    }

    #[test]
    fn pipeline_verified_with_recent_canary() {
        let mut store = StateStore::new();
        let now = Utc::now();
        store.apply_events(&[Event {
            ts: now,
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
        }], now);
        assert!(store.pipeline_verified(now, 300));
    }
}
