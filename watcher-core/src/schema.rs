//! Normalized event schema (design doc §8) and the station-state vocabulary (§6/§4).
//!
//! This module is the ONLY vocabulary the rest of the crate is allowed to know about.
//! No observer-specific field name (e.g. anything from the Claude Code Remote surface)
//! may appear here — see `observer::remote_claude` for the adapter boundary that
//! translates raw provider shapes into these types (§7a).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The six entity kinds from design doc §1a. Never collapsed into each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    /// BAY
    Project,
    /// STATION / WORK CELL — a persistent session. Never an agent.
    Session,
    /// WORKER UNIT — a temporary subagent/dispatch. Never rendered as its own station.
    Agent,
    /// BILLET / JOB PACKET — one unit of assigned work.
    Job,
    /// MACHINE — an autonomous scheduled trigger.
    Routine,
    /// EQUIPMENT — a deterministic process run (tests, scripts). No reasoning loop.
    Check,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityRef {
    pub entity_type: EntityType,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
}

impl EntityRef {
    pub fn new(entity_type: EntityType, id: impl Into<String>) -> Self {
        Self { entity_type, id: id.into(), parent_id: None }
    }
    pub fn with_parent(mut self, parent_id: impl Into<String>) -> Self {
        self.parent_id = Some(parent_id.into());
        self
    }
}

/// The honesty triple (design doc §7a/§8): every derived fact is tagged with how
/// sure we are of it. `Unknown` must never be silently treated as healthy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fidelity {
    Observed,
    Inferred,
    Unknown,
}

/// Normalized event kinds (§8). Deliberately smaller than the brainstormed list —
/// each one must map to something a real observer can actually produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    ProjectDiscovered,
    SessionObserved,
    SessionGone,
    AgentStarted,
    AgentFinished,
    TaskLabelChanged,
    ToolStarted,
    ToolFinished,
    PermissionRequested,
    PermissionCleared,
    TestResult,
    CommitObserved,
    WorktreeChanged,
    DeployStatus,
    CaptureStatus,
    RoutineScheduled,
    RoutineFired,
    RoutineOverdue,
    RateLimitStatus,
    Alert,
    ObserverHealth,
    Heartbeat,
    /// Derived by the reducer only — never emitted directly by an observer.
    StallSuspected,
    /// Derived by the reducer only — never emitted directly by an observer.
    StallConfirmed,
}

/// The twelve §6 health states, plus the four waiting/blocked splits from the
/// pre-build amendments. A station's state is always exactly one of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StationState {
    Working,
    Thinking,
    Specialist,
    WaitingOnAgent,
    WaitingOnSystem,
    Blocked,
    /// The strongest amber on the floor — a pending permission prompt.
    BreyRequired,
    Failed,
    Hung,
    Idle,
    Completed,
    /// Distinct from Idle. Telemetry is missing or too old to trust.
    StaleUnknown,
}

impl StationState {
    /// True for states that must never be silently upgraded to "healthy" —
    /// used by the reducer's absence-is-UNKNOWN rule and the red-team suite.
    pub fn is_attention_state(self) -> bool {
        matches!(
            self,
            StationState::BreyRequired
                | StationState::Failed
                | StationState::Hung
                | StationState::StaleUnknown
        )
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Metrics {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_used: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_max: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unpushed: Option<i64>,
}

/// One normalized event — the only shape the reducer, event log, transport and
/// renderer are allowed to depend on (§8).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub ts: DateTime<Utc>,
    /// Which observer produced this ("remote_claude", "git", "heartbeat", "synthetic_canary", ...).
    pub source: String,
    pub kind: EventKind,
    pub entity: EntityRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Configured model (what the session was created/set to run).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Current model the session is actually set to run (may differ from
    /// `model` after a mid-session switch).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_current: Option<String>,
    /// Model the most recent turn actually ran on — the only place a
    /// turn-scoped fallback (overload/unavailable) shows up.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_last_served: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<StationState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub fidelity: Fidelity,
    #[serde(default)]
    pub metrics: Metrics,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl_secs: Option<i64>,
    /// Routine events only: when the routine is next due, and whether it's
    /// enabled. `None` enabled means "unknown", not "enabled" — the observer
    /// sets this explicitly rather than the reducer guessing a default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_run_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

/// §5a: output velocity, tracked separately from ambient activity. `None` means
/// "no observer for this domain" (absent, not zero) — must never render as a
/// true zero. `Some(0)` means an observed zero.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OutputVelocity {
    pub commits_today: Option<u32>,
    pub tests_passed_today: Option<u32>,
    pub tests_failed_today: Option<u32>,
    pub deploys_today: Option<u32>,
    pub captures_today: Option<u32>,
    pub billets_completed_today: Option<u32>,
    pub last_output_at: Option<DateTime<Utc>>,
    /// Observed vs inferred (e.g. a session-bound routine's success inferred
    /// from post-fire session activity, per §7's routine-outcome caveat).
    pub last_output_fidelity: Option<Fidelity>,
}
