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
        Self {
            value,
            observed_at,
            fidelity,
        }
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
    /// §9/Phase 4E: whatever repo/cwd hint the observer could supply, used
    /// by `bay::BayMap::resolve` to group this session into a project bay.
    pub repo_hint: Option<String>,
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
    /// When `gone` first became true. §16 requires a 60s "fading" grace
    /// state before a station folds down as ENDED — it must never just
    /// vanish from the floor without a trace (adversarial finding #5).
    pub gone_at: Option<DateTime<Utc>>,
    /// S-01: this record's freshness budget. From `Event.ttl_secs`, or
    /// `DEFAULT_SESSION_TTL_SECS` when the observer didn't supply one. If no
    /// fresh `SessionObserved` event lands within this many seconds of
    /// `last_polled_at`, `apply_ttls` marks the record expired/STALE rather
    /// than keeping its last-known state alive forever.
    pub ttl_secs: i64,
    /// S-01 truth rule: true when this record's current values came from a
    /// loaded snapshot (a prior process's state), not a live observation in
    /// THIS process. Restored records must render as STALE/UNKNOWN — never
    /// WORKING/IDLE/healthy — until a fresh event for this entity arrives,
    /// which is when the reducer clears this flag.
    #[serde(default)]
    pub restored: bool,
    #[serde(default)]
    pub restored_at: Option<DateTime<Utc>>,
}

/// S-01 default freshness budget for a session record whose event carried no
/// explicit `ttl_secs` — 15 minutes. Compared against `last_polled_at` (the
/// last time we actually received a fresh observation for this record), not
/// against the session's own `last_activity_at` (which is what stall
/// derivation uses) — TTL is about how long an OBSERVATION stays trustworthy
/// absent a newer one, not about how long the underlying session has been
/// quiet.
pub const DEFAULT_SESSION_TTL_SECS: i64 = 15 * 60;

/// §16's 60s fading-then-ENDED grace window for a session that disappeared
/// from the observer's snapshot.
pub const GONE_FADE_SECS: i64 = 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutineRecord {
    pub id: String,
    pub source: String,
    pub name: String,
    pub bound_session_id: Option<String>,
    pub overdue: bool,
    pub enabled: bool,
    pub next_run_at: Option<DateTime<Utc>>,
    pub prompt_redacted: Option<String>,
    pub last_seen_at: DateTime<Utc>,
    /// True when the routines capability was missing on the most recent
    /// poll — this routine's overdue/enabled/next_run_at fields are frozen
    /// at their last-known values and must not be trusted or rendered as
    /// current (adversarial finding #4: routines were never degraded at
    /// all, so a 2h-dead snapshot still rendered green "ON SCHEDULE").
    pub stale: bool,
    /// S-01: see `SessionRecord::restored`.
    #[serde(default)]
    pub restored: bool,
    #[serde(default)]
    pub restored_at: Option<DateTime<Utc>>,
}

/// A deterministic-process observation (§1a: EQUIPMENT, not a session — no
/// reasoning loop, so no Working/Thinking states, just an observed fact).
/// Currently used by `GitObserver`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckRecord {
    pub id: String,
    pub source: String,
    pub label: Field<String>,
    pub last_seen_at: DateTime<Utc>,
    /// §5a: the underlying event's own timestamp where recoverable (e.g. a
    /// heartbeat's real completion time) — this, not `last_seen_at`, is
    /// what "last output" rollups must use.
    pub last_event_ts: Option<DateTime<Utc>>,
    pub repo_hint: Option<String>,
    /// S-01: see `SessionRecord::restored`.
    #[serde(default)]
    pub restored: bool,
    #[serde(default)]
    pub restored_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PipelineHealth {
    pub last_canary_at: Option<DateTime<Utc>>,
    pub last_any_event_at: Option<DateTime<Utc>>,
    /// S-01: set when a snapshot is loaded at startup; cleared the moment a
    /// fresh (non-canary-sourced) event lands. `pipeline_verified` must stay
    /// false while this is set, even if the canary is ticking and no
    /// observer is reporting Down — a restored floor is not a verified one.
    #[serde(default)]
    pub restored_at: Option<DateTime<Utc>>,
}

/// Phase 4D M-01: whether a machine (a `foundry-agent` process publishing
/// via a `transport::Publisher`) is currently reachable. `Unreachable`
/// covers both "no bundle within `--agent-ttl`" and "the last bundle we DID
/// receive failed verification" — either way this must render as a visible
/// degradation, never a silent drop (§16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MachineStatus {
    Reachable,
    Unreachable,
}

/// One row of the `MACHINES` render section — a snapshot of what
/// `agents::AgentIngestObserver` currently knows about a single agent_id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineRecord {
    pub agent_id: String,
    pub status: MachineStatus,
    pub last_heard_at: Option<DateTime<Utc>>,
    /// Human-readable reason for `Unreachable` (a rejection cause, or
    /// "agent unreachable" for a plain TTL expiry). `None` when `Reachable`.
    pub reason: Option<String>,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StateStore {
    pub sessions: BTreeMap<String, SessionRecord>,
    pub routines: BTreeMap<String, RoutineRecord>,
    pub checks: BTreeMap<String, CheckRecord>,
    pub observer_health: BTreeMap<String, ObserverHealth>,
    pub pipeline: PipelineHealth,
    /// Phase 4D M-01: the current `MACHINES` view, keyed by agent_id.
    /// Replaced wholesale each poll by `set_machines` — this is a live
    /// status snapshot, not an event-sourced record, so it is cleared (not
    /// restored) across a snapshot load, matching `observer_health`.
    #[serde(default)]
    pub machines: BTreeMap<String, MachineRecord>,
    /// F-11: the last-accepted `seq` this process has ever seen from each
    /// agent_id (`agents::AgentIngestObserver::seq_watermarks()`). Unlike
    /// `machines`/`observer_health`, this MUST survive a restart — it is
    /// exactly what closes the replay window a fresh, watermark-less
    /// `AgentIngestObserver` would otherwise reopen for every known agent
    /// (the same class of bug as F-3, just re-triggered by a restart
    /// instead of a bare `seq == 0`). `persist::mark_restored` deliberately
    /// does NOT clear this field.
    #[serde(default)]
    pub agent_seq_watermarks: BTreeMap<String, u64>,
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
                    // S-01/F-5: only the in-process synthetic canary may
                    // certify the local poll loop as alive — a Heartbeat
                    // event forwarded from anywhere else (a remote agent
                    // bundle, say) proves nothing about THIS process's own
                    // loop and must not mint `last_canary_at`.
                    if ev.source == crate::observer::CANARY_SOURCE {
                        self.pipeline.last_canary_at = Some(ev.ts);
                    }
                }
                EventKind::SessionObserved => {
                    self.maybe_clear_restored_at(ev, now);
                    self.apply_session_observed(ev, now);
                }
                EventKind::SessionGone => {
                    if let Some(rec) = self.sessions.get_mut(&ev.entity.id) {
                        if !rec.gone {
                            rec.gone_at = Some(now);
                        }
                        rec.gone = true;
                        rec.restored = false;
                        rec.restored_at = None;
                        rec.displayed_state =
                            Field::new(StationState::StaleUnknown, now, Fidelity::Observed);
                    }
                }
                EventKind::RoutineScheduled | EventKind::RoutineOverdue => {
                    self.apply_routine(ev, now, ev.kind == EventKind::RoutineOverdue);
                }
                EventKind::WorktreeChanged => {
                    self.maybe_clear_restored_at(ev, now);
                    self.apply_check(ev, now);
                }
                _ => {}
            }
        }
        self.derive_stalls(now);
        self.apply_ttls(now);
    }

    /// S-01/F-5 rule (kept in one place, not duplicated at each call site):
    /// `pipeline.restored_at` is only cleared by a LOCAL, entity-bearing
    /// observation — `SessionObserved` or `WorktreeChanged` — from a source
    /// that isn't itself a coarse liveness signal (`heartbeat`) or the
    /// synthetic canary. Rationale: proving the floor is live again requires
    /// a real observer to have re-observed a real entity in THIS process,
    /// not merely a heartbeat-shaped ping (which, like the canary, only
    /// proves *something* is alive, not that any entity was re-observed).
    fn maybe_clear_restored_at(&mut self, ev: &Event, now: DateTime<Utc>) {
        if ev.source == "heartbeat" || ev.source == crate::observer::CANARY_SOURCE {
            return;
        }
        // F-1: a replayed/backfilled event whose own timestamp already lies
        // outside its trust window is not proof the floor is live again —
        // only an observation that is itself fresh (by the same TTL rule
        // `apply_session_observed`/`apply_ttls` use) may certify that.
        let ttl = ev.ttl_secs.unwrap_or(DEFAULT_SESSION_TTL_SECS);
        let age = (now - ev.ts).num_seconds();
        if ttl <= 0 || age <= ttl {
            self.pipeline.restored_at = None;
        }
    }

    /// S-01 point 4: an observation older than its TTL must not keep the
    /// record's last-reported state alive forever — mark it expired/STALE.
    /// Compared against `last_polled_at` (last time we actually received a
    /// fresh observation), not `last_activity_at` (the session's own
    /// activity clock, used separately by `derive_stalls`) — this is about
    /// how long an OBSERVATION stays trustworthy absent a newer one.
    fn apply_ttls(&mut self, now: DateTime<Utc>) {
        for rec in self.sessions.values_mut() {
            if rec.gone {
                continue;
            }
            if rec.ttl_secs <= 0 {
                continue;
            }
            let age = (now - rec.last_polled_at).num_seconds();
            if age > rec.ttl_secs {
                rec.displayed_state =
                    Field::new(StationState::StaleUnknown, now, Fidelity::Unknown);
            }
        }
    }

    fn apply_session_observed(&mut self, ev: &Event, now: DateTime<Utc>) {
        let Some(state) = ev.state else { return };
        // Derive the session's real last-activity time from the observer's
        // elapsed_ms metric (itself computed from the provider's updated_at),
        // NOT from `now` — `now` is only when WE polled, which says nothing
        // about whether the session actually did anything since last time.
        // `None` means the observer could not establish a real timestamp
        // (missing/unparseable/clock-skewed). CRITICAL: on an EXISTING
        // record this must NOT fall back to `now` — doing so was an actual
        // bug (adversarial finding #2): every poll would re-anchor the
        // staleness clock to "just now", so a session with no real
        // timestamp data could never be flagged Hung no matter how many
        // hours passed. `now` is only an acceptable fallback the very first
        // time we ever see a session (nothing better to anchor to yet).
        let activity_at = ev
            .metrics
            .elapsed_ms
            .map(|ms| now - chrono::Duration::milliseconds(ms));

        // S-01/F-1: `last_polled_at` must be when this observation actually
        // happened (`ev.ts`), not merely when THIS process happened to fold
        // it in. Feeding a historical event log entry through `apply_events`
        // at restart calls this with `now` = the restart's wall clock, which
        // is hours or days after `ev.ts` for a replayed entry — using `now`
        // here would re-anchor the record's freshness budget to "just
        // observed", letting a day-old logged WORKING session render live
        // again purely from being replayed. Never let a future-dated `ev.ts`
        // count as "more recent than now" either.
        let polled_at = ev.ts.min(now);
        let ttl = ev.ttl_secs.unwrap_or(DEFAULT_SESSION_TTL_SECS);
        // A replayed/backfilled event whose own timestamp is already outside
        // its TTL window is not a fresh live observation — it must not clear
        // `restored`/`restored_at` (that would un-mark a genuinely stale
        // restored record as live) even though it still folds its state in
        // for `apply_ttls` to immediately re-expire.
        let is_fresh = ttl <= 0 || (now - polled_at).num_seconds() <= ttl;

        let entry = self
            .sessions
            .entry(ev.entity.id.clone())
            .or_insert_with(|| SessionRecord {
                id: ev.entity.id.clone(),
                source: ev.source.clone(),
                repo_hint: ev.entity.parent_id.clone(),
                observed_state: Field::new(state, now, ev.fidelity),
                displayed_state: Field::new(state, now, ev.fidelity),
                model: None,
                model_current: None,
                model_last_served: None,
                label: None,
                session_kind: None,
                last_polled_at: polled_at,
                last_activity_at: activity_at.unwrap_or(polled_at),
                stall_warning: false,
                gone: false,
                gone_at: None,
                ttl_secs: ttl,
                restored: false,
                restored_at: None,
            });
        entry.observed_state = Field::new(state, now, ev.fidelity);
        entry.displayed_state = Field::new(state, now, ev.fidelity);
        entry.source = ev.source.clone();
        entry.last_polled_at = polled_at;
        if let Some(a) = activity_at {
            entry.last_activity_at = a;
        }
        entry.gone = false;
        entry.gone_at = None;
        entry.ttl_secs = ttl;
        // S-01: a fresh live observation for this entity means it is no
        // longer a stale carry-over from a loaded snapshot — but a replayed
        // event that is itself already stale (see `is_fresh` above) must
        // leave `restored`/`restored_at` exactly as they were.
        if is_fresh {
            entry.restored = false;
            entry.restored_at = None;
        }
        if ev.entity.parent_id.is_some() {
            entry.repo_hint = ev.entity.parent_id.clone();
        }
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
        // §16/schema.rs contract: `None` enabled means UNKNOWN, not enabled.
        // Defaulting to `true` (as this used to) would render a routine
        // whose enabled-state we can't confirm as a confident green
        // "ON SCHEDULE" — the safe default is `false` (adversarial finding #4).
        let entry = self
            .routines
            .entry(ev.entity.id.clone())
            .or_insert_with(|| RoutineRecord {
                id: ev.entity.id.clone(),
                source: ev.source.clone(),
                name: ev.label.clone().unwrap_or_default(),
                bound_session_id: ev.session_id.clone(),
                overdue,
                enabled: ev.enabled.unwrap_or(false),
                next_run_at: ev.next_run_at,
                prompt_redacted: ev.detail.clone(),
                last_seen_at: now,
                stale: false,
                restored: false,
                restored_at: None,
            });
        entry.source = ev.source.clone();
        entry.name = ev.label.clone().unwrap_or_else(|| entry.name.clone());
        entry.bound_session_id = ev.session_id.clone().or(entry.bound_session_id.clone());
        entry.overdue = overdue;
        entry.enabled = ev.enabled.unwrap_or(entry.enabled);
        entry.next_run_at = ev.next_run_at.or(entry.next_run_at);
        entry.prompt_redacted = ev.detail.clone().or(entry.prompt_redacted.clone());
        entry.restored = false;
        entry.restored_at = None;
        entry.last_seen_at = now;
        entry.stale = false;
    }

    fn apply_check(&mut self, ev: &Event, now: DateTime<Utc>) {
        let Some(label) = &ev.label else { return };
        // §5a output velocity: the event's OWN timestamp (recovered from
        // elapsed_ms, exactly like session activity_at) when available —
        // NOT poll time, which would make "last output" always read as
        // "just now" regardless of when the underlying work actually
        // finished (the same class of bug as finding #2).
        let event_ts = ev
            .metrics
            .elapsed_ms
            .map(|ms| now - chrono::Duration::milliseconds(ms));
        let entry = self
            .checks
            .entry(ev.entity.id.clone())
            .or_insert_with(|| CheckRecord {
                id: ev.entity.id.clone(),
                source: ev.source.clone(),
                label: Field::new(label.clone(), now, ev.fidelity),
                last_seen_at: now,
                last_event_ts: event_ts,
                repo_hint: ev.entity.parent_id.clone(),
                restored: false,
                restored_at: None,
            });
        entry.source = ev.source.clone();
        entry.label = Field::new(label.clone(), now, ev.fidelity);
        if let Some(t) = event_ts {
            entry.last_event_ts = Some(t);
        }
        if ev.entity.parent_id.is_some() {
            entry.repo_hint = ev.entity.parent_id.clone();
        }
        entry.last_seen_at = now;
        entry.restored = false;
        entry.restored_at = None;
    }

    /// Rule 2: STALL derivation. Only sessions the observer reports as
    /// Working can go Hung — everything else is already an intentional
    /// "stopped" state and hanging is not a meaningful concept for it.
    fn derive_stalls(&mut self, now: DateTime<Utc>) {
        for rec in self.sessions.values_mut() {
            if rec.gone {
                continue;
            }
            // S-01: a record still carrying `observed_state: Working` from
            // BEFORE a restart must not get re-derived into Hung here — it
            // must stay the honest StaleUnknown `persist::mark_restored` set
            // it to, until a fresh event actually re-observes it (which is
            // exactly what clears `restored`).
            if rec.restored {
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
    /// after `observer.poll()`, passing its current health.
    ///
    /// Degradation is CAPABILITY-SPECIFIC, not just an overall status check
    /// (adversarial finding #3/#4, both from the same root cause): an
    /// observer that still supplies routines but has lost sessions must
    /// degrade sessions WITHOUT falsely also degrading routines that are
    /// still fine, and vice versa. A blanket "status != Healthy degrades
    /// everything" rule gets both directions wrong — it can under-degrade
    /// (miss a partial loss the aggregate status doesn't reflect) or
    /// over-degrade (punish data that's still genuinely fresh). So this
    /// checks `health.capabilities` per capability instead of trusting the
    /// single rolled-up `status` enum. It does NOT leave anything frozen at
    /// its last-good value, which would silently look healthy forever.
    /// `sessions_cap`/`routines_cap`: the capability NAME this specific
    /// observer uses to say "I can supply session/routine records" — pass
    /// `None` for an observer that never sources that kind of record at
    /// all (e.g. `git` sources neither). Different observers legitimately
    /// use different capability names for the same kind of record (e.g.
    /// `remote_claude` uses "sessions", `local_claude` uses
    /// "local_sessions") — hardcoding a single name here was itself a bug
    /// caught while wiring up Phase 3.5's second sessions-sourcing
    /// observer: it made local_claude's sessions look permanently degraded
    /// because it (correctly) never reports the *other* observer's
    /// capability name.
    pub fn apply_observer_health(
        &mut self,
        health: &ObserverHealth,
        now: DateTime<Utc>,
        sessions_cap: Option<&str>,
        routines_cap: Option<&str>,
    ) {
        let sessions_lost = sessions_cap.map(|cap| !health.capabilities.has(cap));
        let routines_lost = routines_cap.map(|cap| !health.capabilities.has(cap));
        self.observer_health
            .insert(health.name.clone(), health.clone());

        if let Some(true) = sessions_lost {
            for rec in self.sessions.values_mut() {
                if rec.source == health.name {
                    rec.displayed_state =
                        Field::new(StationState::StaleUnknown, now, Fidelity::Unknown);
                }
            }
        }
        if let Some(lost) = routines_lost {
            for rec in self.routines.values_mut() {
                if rec.source == health.name {
                    rec.stale = lost;
                }
            }
        }
    }

    /// §16 "false everything-healthy" defense: if no canary heartbeat has
    /// landed recently, the floor must NOT be trusted, no matter how good the
    /// last snapshot looked. The canary alone is NOT sufficient, though — it
    /// only proves the poll loop itself is alive, in-process, unconditionally
    /// (adversarial finding: a fully DOWN `remote_claude` observer still let
    /// the canary carry "pipeline verified" while every session was blind).
    /// So this also requires no *real* (non-canary) observer to be Down —
    /// the top-line claim must reflect the data sources, not just the loop.
    pub fn pipeline_verified(&self, now: DateTime<Utc>, max_canary_age_secs: i64) -> bool {
        let canary_fresh = match self.pipeline.last_canary_at {
            Some(t) => (now - t).num_seconds() <= max_canary_age_secs,
            None => false,
        };
        // S-01: a floor freshly loaded from a snapshot must not read as
        // verified just because the canary is ticking and no observer
        // happens to be Down — the canary AND at least one real observer
        // must both have produced a fresh event in THIS process first.
        canary_fresh
            && !self.any_real_observer_down()
            && !self.any_real_observer_blind()
            && self.pipeline.restored_at.is_none()
    }

    /// True if any observer other than the synthetic canary itself has gone
    /// Down. Used by `pipeline_verified` and available for the renderer to
    /// name which observer broke the top-line claim.
    pub fn any_real_observer_down(&self) -> bool {
        self.observer_health
            .values()
            .any(|h| h.name != "synthetic_canary" && matches!(h.status, ObserverStatus::Down))
    }

    /// F-4: true if any real (non-canary) observer confirmed NOTHING this
    /// poll — an empty `capabilities` set, whether the aggregate `status` is
    /// `Down` (3 consecutive failures) or merely `Degraded` (1-2). A
    /// Degraded-but-blind observer (e.g. two consecutive failures, still
    /// short of Down) is just as unable to back up "pipeline verified" as a
    /// fully Down one — `status` alone under-reports this.
    pub fn any_real_observer_blind(&self) -> bool {
        self.observer_health
            .values()
            .any(|h| h.name != "synthetic_canary" && h.capabilities.0.is_empty())
    }

    /// Replaces the whole `MACHINES` view for this poll — called with
    /// `AgentIngestObserver::machines(now)`'s freshly-computed rows.
    pub fn set_machines(&mut self, rows: Vec<MachineRecord>) {
        self.machines = rows.into_iter().map(|r| (r.agent_id.clone(), r)).collect();
    }

    /// F-11: records this poll's `agents::AgentIngestObserver::seq_watermarks()`
    /// so the next `save_snapshot` persists it and a restart can seed the
    /// observer's replay guard via `restore_seq_watermarks` instead of
    /// starting blank.
    pub fn set_agent_seq_watermarks(&mut self, watermarks: BTreeMap<String, u64>) {
        self.agent_seq_watermarks = watermarks;
    }

    pub fn last_sync_age_secs(&self, now: DateTime<Utc>) -> Option<i64> {
        self.pipeline
            .last_any_event_at
            .map(|t| (now - t).num_seconds().max(0))
    }

    /// §5a output velocity, first cut: the most recent heartbeat-sourced
    /// check's own event timestamp — a real completed-script signal, not
    /// ambient activity. `None` means genuinely no output observer has
    /// reported anything yet (absence, not a fabricated zero) — the
    /// renderer must show that as "n/a", never as "0s ago".
    /// APPROXIMATION: does not yet distinguish ok/degraded/escalate status,
    /// or fold in git commits — first-cut rollup, not a final taxonomy.
    pub fn last_output_at(&self) -> Option<DateTime<Utc>> {
        self.checks
            .values()
            .filter(|c| c.source == "heartbeat")
            .filter_map(|c| c.last_event_ts)
            .max()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observer::{CAP_ROUTINES, CAP_SESSIONS};
    use crate::schema::{EntityRef, EntityType, Metrics};

    fn session_event(
        id: &str,
        state: StationState,
        fidelity: Fidelity,
        ts: DateTime<Utc>,
    ) -> Event {
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
        store.apply_events(
            &[session_event(
                "s1",
                StationState::BreyRequired,
                Fidelity::Observed,
                now,
            )],
            now,
        );
        let rec = &store.sessions["s1"];
        assert_eq!(rec.displayed_state.value, StationState::BreyRequired);
    }

    #[test]
    fn working_session_goes_hung_after_threshold_with_inferred_fidelity() {
        let mut store = StateStore::new();
        let t0 = Utc::now();
        store.apply_events(
            &[session_event(
                "s1",
                StationState::Working,
                Fidelity::Observed,
                t0,
            )],
            t0,
        );
        assert_eq!(
            store.sessions["s1"].displayed_state.value,
            StationState::Working
        );

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
        store.apply_events(
            &[session_event(
                "s1",
                StationState::Working,
                Fidelity::Observed,
                t0,
            )],
            t0,
        );
        let warn_time = t0 + chrono::Duration::seconds((STALL_CONFIRM_SECS as f64 * 0.7) as i64);
        store.derive_stalls(warn_time);
        let rec = &store.sessions["s1"];
        assert!(rec.stall_warning);
        assert_eq!(
            rec.displayed_state.value,
            StationState::Working,
            "still Working, only warning, below full threshold"
        );
    }

    #[test]
    fn non_working_states_never_derive_hung() {
        let mut store = StateStore::new();
        let t0 = Utc::now();
        store.apply_events(
            &[session_event(
                "s1",
                StationState::Idle,
                Fidelity::Observed,
                t0,
            )],
            t0,
        );
        let later = t0 + chrono::Duration::seconds(STALL_CONFIRM_SECS * 10);
        store.derive_stalls(later);
        assert_eq!(
            store.sessions["s1"].displayed_state.value,
            StationState::Idle
        );
    }

    #[test]
    fn observer_going_down_degrades_its_sessions_to_stale_not_frozen_healthy() {
        let mut store = StateStore::new();
        let t0 = Utc::now();
        store.apply_events(
            &[session_event(
                "s1",
                StationState::Working,
                Fidelity::Observed,
                t0,
            )],
            t0,
        );
        assert_eq!(
            store.sessions["s1"].displayed_state.value,
            StationState::Working
        );

        let mut health = ObserverHealth::new("remote_claude");
        health.record_failure("e1");
        health.record_failure("e2");
        health.record_failure("e3"); // -> Down
        assert_eq!(health.status, ObserverStatus::Down);

        store.apply_observer_health(
            &health,
            t0 + chrono::Duration::seconds(30),
            Some(CAP_SESSIONS),
            Some(CAP_ROUTINES),
        );
        assert_eq!(
            store.sessions["s1"].displayed_state.value,
            StationState::StaleUnknown
        );
        assert_eq!(
            store.sessions["s1"].displayed_state.fidelity,
            Fidelity::Unknown
        );
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
        store.apply_events(
            &[Event {
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
            }],
            now,
        );
        assert!(store.pipeline_verified(now, 300));
    }
}
