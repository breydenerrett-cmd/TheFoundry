//! Observer trait + the strict `RemoteClaudeObserver` adapter boundary (§7a).
//!
//! CRITICAL RULE: no field name, endpoint name, or enum string from the
//! Claude Code Remote surface may leak outside `remote_claude_raw` /
//! `remote_claude` below. Everything past `normalize()` speaks only
//! `schema::Event`. A grep for `status_bucket`, `worktree_state`,
//! `post_turn_summary`, `pending_action` etc. anywhere else in this crate is
//! a lint failure by design.
//!
//! DATA-ACCESS OPEN QUESTION (flagged for Brey, see PHASE1_REPORT.md and the
//! Phase 3 report): the Claude Code Remote surface, as probed in Phase 1, is
//! only reachable via MCP tool calls inside an authenticated Claude session —
//! there is no documented public REST endpoint this Rust binary can call
//! directly. For Phase 1-3 the bridge is a well-known local directory of
//! redacted raw-snapshot JSON files (`live-feed/`), refreshed by a Claude
//! session that has the Claude_Code_Remote MCP tools loaded. This keeps the
//! adapter's *contract* (raw snapshot in, normalized events out) identical to
//! whatever the eventual real bridge turns out to be — a subprocess wrapping
//! a headless `claude` invocation, a future public API, or something else —
//! without baking that undecided mechanism into the rest of the crate.

use crate::health::{CapabilitySet, ObserverHealth};
use crate::schema::{Event, EventKind, EntityRef, EntityType, Fidelity, Metrics, StationState};
use chrono::{DateTime, Utc};
use std::fs;
use std::path::{Path, PathBuf};

pub trait Observer {
    /// Stable name used as `Event.source` and in health/degradation reporting.
    fn name(&self) -> &str;

    /// Poll for new normalized events. MUST NOT throw the whole watcher over —
    /// any internal failure is captured and reflected in `health()`, never
    /// propagated as a panic that takes other observers down with it.
    fn poll(&mut self, now: DateTime<Utc>) -> Vec<Event>;

    fn health(&self) -> &ObserverHealth;
}

/// Raw, per-provider snapshot shapes live in this submodule ONLY. This is the
/// one place allowed to know what the Remote surface's JSON looks like.
mod remote_claude_raw {
    use serde::Deserialize;

    #[derive(Debug, Deserialize, Default)]
    pub struct SessionsSnapshot {
        #[serde(default)]
        pub data: Vec<RawSession>,
    }

    #[derive(Debug, Deserialize, Default)]
    pub struct RawSession {
        pub id: String,
        #[serde(default)]
        pub title: Option<String>,
        #[serde(default)]
        pub environment_kind: Option<String>,
        #[serde(default)]
        pub origin: Option<String>,
        #[serde(default)]
        pub connection_status: Option<String>,
        #[serde(default)]
        pub status_bucket: Option<String>,
        #[serde(default)]
        pub configured_model: Option<String>,
        #[serde(default)]
        pub session_context: Option<RawSessionContext>,
        #[serde(default)]
        pub external_metadata: Option<RawExternalMetadata>,
        #[serde(default)]
        pub created_at: Option<String>,
        #[serde(default)]
        pub updated_at: Option<String>,
        #[serde(default)]
        pub post_turn_summary: Option<RawPostTurnSummary>,
        #[serde(default)]
        pub last_init_error: Option<RawInitError>,
        #[serde(default)]
        pub tags: Vec<String>,
    }

    #[derive(Debug, Deserialize, Default)]
    pub struct RawSessionContext {
        #[serde(default)]
        pub model: Option<String>,
    }

    #[derive(Debug, Deserialize, Default)]
    pub struct RawExternalMetadata {
        #[serde(default)]
        pub last_served_model: Option<String>,
    }

    #[derive(Debug, Deserialize, Default)]
    pub struct RawPostTurnSummary {
        #[serde(default)]
        pub status_category: Option<String>,
        #[serde(default)]
        pub status_detail: Option<String>,
        #[serde(default)]
        pub needs_action: Option<String>,
    }

    #[derive(Debug, Deserialize, Default)]
    pub struct RawInitError {
        #[serde(default)]
        pub error_kind: Option<String>,
        #[serde(default)]
        pub recoverable: Option<bool>,
    }

    #[derive(Debug, Deserialize, Default)]
    pub struct TriggersSnapshot {
        #[serde(default)]
        pub data: Vec<RawTrigger>,
    }

    #[derive(Debug, Deserialize, Default)]
    pub struct RawTrigger {
        pub id: String,
        pub name: String,
        #[serde(default)]
        pub cron_expression: Option<String>,
        #[serde(default)]
        pub enabled: Option<bool>,
        #[serde(default)]
        pub next_run_at: Option<String>,
        #[serde(default)]
        pub last_fired_at: Option<String>,
        #[serde(default)]
        pub persistent_session_id: Option<String>,
        /// Already redacted by the bridge per §15 — never the raw prompt.
        #[serde(default)]
        pub prompt_redacted: Option<String>,
    }
}

/// Normalized capability names this observer can, in principle, supply.
pub const CAP_SESSIONS: &str = "sessions";
pub const CAP_PERMISSIONS: &str = "permissions";
pub const CAP_WORKTREE: &str = "worktree";
pub const CAP_CONTEXT_USAGE: &str = "context_usage";
pub const CAP_ROUTINES: &str = "routines";

/// Reads redacted raw snapshots dropped into `feed_dir` by a bridge process
/// (today: a Claude session with the Claude_Code_Remote MCP tools loaded).
/// Expected files: `list_sessions.json`, `list_triggers.json` (same shape as
/// the Phase 1 fixture corpus). Missing/unreadable files degrade capability
/// rather than erroring the whole observer.
pub struct RemoteClaudeObserver {
    feed_dir: PathBuf,
    health: ObserverHealth,
    known_session_ids: std::collections::BTreeSet<String>,
    known_trigger_next_run: std::collections::BTreeMap<String, DateTime<Utc>>,
}

impl RemoteClaudeObserver {
    pub fn new(feed_dir: impl AsRef<Path>) -> Self {
        Self {
            feed_dir: feed_dir.as_ref().to_path_buf(),
            health: ObserverHealth::new("remote_claude"),
            known_session_ids: Default::default(),
            known_trigger_next_run: Default::default(),
        }
    }

    fn read_json<T: serde::de::DeserializeOwned + Default>(&self, filename: &str) -> Option<T> {
        let path = self.feed_dir.join(filename);
        let bytes = fs::read(&path).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    fn map_station_state(raw: &remote_claude_raw::RawSession) -> (StationState, Fidelity) {
        // §7a: unknown-enum path degrades rather than throws.
        if raw.connection_status.as_deref() == Some("disconnected") {
            if let Some(err) = &raw.last_init_error {
                if err.error_kind.as_deref() == Some("computer_unreachable") {
                    return (StationState::StaleUnknown, Fidelity::Observed);
                }
            }
        }
        if let Some(pts) = &raw.post_turn_summary {
            if pts.status_category.as_deref() == Some("need_input") || pts.needs_action.is_some() {
                return (StationState::BreyRequired, Fidelity::Observed);
            }
        }
        match raw.status_bucket.as_deref() {
            Some("SESSION_STATUS_BUCKET_WORKING") => (StationState::Working, Fidelity::Observed),
            Some("SESSION_STATUS_BUCKET_BLOCKED") => (StationState::Blocked, Fidelity::Observed),
            Some("SESSION_STATUS_BUCKET_REVIEW_READY") => (StationState::Idle, Fidelity::Observed),
            Some("SESSION_STATUS_BUCKET_COMPLETED") => (StationState::Completed, Fidelity::Observed),
            Some(_other) => (StationState::StaleUnknown, Fidelity::Unknown),
            None => (StationState::StaleUnknown, Fidelity::Unknown),
        }
    }

    fn parse_ts(s: &Option<String>) -> Option<DateTime<Utc>> {
        s.as_ref().and_then(|v| DateTime::parse_from_rfc3339(v).ok()).map(|d| d.with_timezone(&Utc))
    }
}

impl Observer for RemoteClaudeObserver {
    fn name(&self) -> &str {
        "remote_claude"
    }

    fn poll(&mut self, now: DateTime<Utc>) -> Vec<Event> {
        let mut events = Vec::new();
        let mut caps = CapabilitySet::new();

        if let Some(snap) = self.read_json::<remote_claude_raw::SessionsSnapshot>("list_sessions.json") {
            caps.0.insert(CAP_SESSIONS.to_string());
            let mut seen_this_poll = std::collections::BTreeSet::new();
            for raw in &snap.data {
                seen_this_poll.insert(raw.id.clone());
                self.known_session_ids.insert(raw.id.clone());

                let (state, fidelity) = Self::map_station_state(raw);
                if state == StationState::BreyRequired {
                    caps.0.insert(CAP_PERMISSIONS.to_string());
                }

                let updated_at = Self::parse_ts(&raw.updated_at).unwrap_or(now);
                let elapsed_ms = (now - updated_at).num_milliseconds().max(0);

                events.push(Event {
                    ts: now,
                    source: self.name().to_string(),
                    kind: EventKind::SessionObserved,
                    entity: EntityRef::new(EntityType::Session, raw.id.clone()),
                    project_id: None,
                    session_id: Some(raw.id.clone()),
                    model: raw.configured_model.clone(),
                    model_current: raw.session_context.as_ref().and_then(|c| c.model.clone()),
                    model_last_served: raw.external_metadata.as_ref().and_then(|m| m.last_served_model.clone()),
                    effort: None,
                    state: Some(state),
                    label: raw.post_turn_summary.as_ref().and_then(|p| p.status_detail.clone())
                        .or_else(|| raw.title.clone()),
                    detail: raw.environment_kind.clone(),
                    fidelity,
                    metrics: Metrics { elapsed_ms: Some(elapsed_ms), ..Default::default() },
                    ttl_secs: Some(120),
                    next_run_at: None,
                    enabled: None,
                });
            }
            // Sessions previously seen but absent from this snapshot: SessionGone.
            for old_id in self.known_session_ids.difference(&seen_this_poll).cloned().collect::<Vec<_>>() {
                events.push(Event {
                    ts: now,
                    source: self.name().to_string(),
                    kind: EventKind::SessionGone,
                    entity: EntityRef::new(EntityType::Session, old_id.clone()),
                    project_id: None,
                    session_id: Some(old_id),
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
                });
            }
        }

        if let Some(snap) = self.read_json::<remote_claude_raw::TriggersSnapshot>("list_triggers.json") {
            caps.0.insert(CAP_ROUTINES.to_string());
            for raw in &snap.data {
                let next_run = Self::parse_ts(&raw.next_run_at);
                // A disabled routine is never "overdue" — it's deliberately off, not stuck.
                let is_overdue = raw.enabled.unwrap_or(true)
                    && next_run.map(|nr| now > nr + chrono::Duration::minutes(5)).unwrap_or(false);
                if let Some(nr) = next_run {
                    self.known_trigger_next_run.insert(raw.id.clone(), nr);
                }

                events.push(Event {
                    ts: now,
                    source: self.name().to_string(),
                    kind: if is_overdue { EventKind::RoutineOverdue } else { EventKind::RoutineScheduled },
                    entity: EntityRef::new(EntityType::Routine, raw.id.clone()),
                    project_id: None,
                    session_id: raw.persistent_session_id.clone(),
                    model: None,
                    model_current: None,
                    model_last_served: None,
                    effort: None,
                    state: None,
                    label: Some(raw.name.clone()),
                    // prompt_redacted is already truncated by the bridge — never the raw prompt (§15).
                    detail: raw.prompt_redacted.clone(),
                    fidelity: if raw.enabled.unwrap_or(true) { Fidelity::Observed } else { Fidelity::Observed },
                    metrics: Metrics::default(),
                    ttl_secs: Some(600),
                    next_run_at: next_run,
                    enabled: raw.enabled,
                });
            }
        }

        if caps.0.is_empty() {
            self.health.record_failure("no readable snapshot files in feed_dir");
        } else {
            self.health.record_success(now, caps);
        }

        events
    }

    fn health(&self) -> &ObserverHealth {
        &self.health
    }
}

/// A synthetic canary observer (§16 "false everything-healthy" defense): emits
/// a heartbeat every poll. If the reducer stops seeing fresh canary events,
/// the whole pipeline — not just one observer — is declared unverified.
pub struct SyntheticCanary {
    health: ObserverHealth,
}

impl SyntheticCanary {
    pub fn new() -> Self {
        Self { health: ObserverHealth::new("synthetic_canary") }
    }
}

impl Default for SyntheticCanary {
    fn default() -> Self {
        Self::new()
    }
}

impl Observer for SyntheticCanary {
    fn name(&self) -> &str {
        "synthetic_canary"
    }

    fn poll(&mut self, now: DateTime<Utc>) -> Vec<Event> {
        self.health.record_success(now, CapabilitySet::from_iter(["heartbeat"]));
        vec![Event {
            ts: now,
            source: self.name().to_string(),
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
        }]
    }

    fn health(&self) -> &ObserverHealth {
        &self.health
    }
}
