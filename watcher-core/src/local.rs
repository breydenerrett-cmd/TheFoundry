//! Phase 3.5 — zero-model-token, standalone-capable observers.
//!
//! These are things a plain Rust binary can read TODAY, with no Claude
//! session, no model turn, and no authenticated Remote API involved — see
//! `PHASE3_5_ACCESS_BRIDGE.md` for the investigation that led here and why
//! this is the ceiling of what's currently possible without one.
//!
//! `LocalClaudeObserver` shells out to the real, documented `claude agents
//! --json` CLI flag — confirmed empirically (see the report) to be a
//! deterministic, sub-second, zero-token command. It only ever sees Claude
//! Code processes running on THIS machine, never bridge/cloud sessions
//! elsewhere — that scope limit is fundamental, not a bug.
//!
//! `GitObserver` shells out to plain `git`, exactly as §8's data-architecture
//! diagram always intended (a `git` observer alongside the Claude one).

use crate::health::{CapabilitySet, ObserverHealth};
use crate::observer::Observer;
use crate::schema::{EntityRef, EntityType, Event, EventKind, Fidelity, Metrics, StationState};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

pub const CAP_LOCAL_SESSIONS: &str = "local_sessions";
pub const CAP_GIT: &str = "git";

#[derive(Debug, Deserialize)]
struct LocalAgentEntry {
    #[allow(dead_code)]
    pid: u64,
    cwd: String,
    kind: String,
    #[allow(dead_code)]
    #[serde(rename = "startedAt")]
    started_at: i64,
    #[serde(rename = "sessionId")]
    session_id: String,
    name: Option<String>,
    /// Present for `--bg` background sessions, absent for the plain
    /// interactive entry. When present, it's a real activity signal beyond
    /// mere liveness — upgrades fidelity from Inferred to Observed.
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    state: Option<String>,
}

/// Wraps `claude agents --json` — the ONLY confirmed zero-token, deterministic,
/// scriptable session-discovery mechanism found in the Phase 3.5 investigation.
/// Local-machine scope only; see the module doc above.
pub struct LocalClaudeObserver {
    health: ObserverHealth,
    known_ids: BTreeSet<String>,
    claude_bin: String,
}

impl LocalClaudeObserver {
    pub fn new() -> Self {
        Self {
            health: ObserverHealth::new("local_claude"),
            known_ids: BTreeSet::new(),
            claude_bin: "claude".to_string(),
        }
    }
}

impl Default for LocalClaudeObserver {
    fn default() -> Self {
        Self::new()
    }
}

fn empty_event(ts: DateTime<Utc>, source: &str, kind: EventKind, entity: EntityRef) -> Event {
    Event {
        ts,
        source: source.to_string(),
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

impl Observer for LocalClaudeObserver {
    fn name(&self) -> &str {
        "local_claude"
    }

    fn poll(&mut self, now: DateTime<Utc>) -> Vec<Event> {
        let mut events = Vec::new();
        let output = Command::new(&self.claude_bin).arg("agents").arg("--json").output();

        let entries: Option<Vec<LocalAgentEntry>> = match output {
            Ok(out) if out.status.success() => serde_json::from_slice(&out.stdout).ok(),
            _ => None,
        };

        match entries {
            Some(list) => {
                let mut seen = BTreeSet::new();
                for e in &list {
                    seen.insert(e.session_id.clone());
                    self.known_ids.insert(e.session_id.clone());

                    let raw_label = format!("{} [{}] {}", e.name.clone().unwrap_or_default(), e.kind, e.cwd);

                    let mut ev = empty_event(now, self.name(), EventKind::SessionObserved, EntityRef::new(EntityType::Session, e.session_id.clone()));
                    ev.session_id = Some(e.session_id.clone());
                    // Honest limitation: `claude agents --json` proves the process
                    // is ALIVE, not what it's doing. We render that as Working
                    // with Inferred fidelity — a real fact (process liveness),
                    // deliberately not claiming precision we don't have. This is
                    // the same coarse-telemetry honesty rule already applied to
                    // cloud sessions elsewhere in this crate.
                    //
                    // elapsed_ms is deliberately left None (not `startedAt`-based):
                    // this only proves the process started at that time, not that
                    // it's been idle since — a long-running, genuinely-active local
                    // session would otherwise get falsely flagged Hung the moment
                    // its process age crossed the stall threshold, guaranteed for
                    // every session that runs more than ~25 minutes. Leaving it
                    // None makes the reducer anchor staleness to when FOUNDRY first
                    // saw it (finding #2's "don't fabricate freshness" rule),
                    // which is the honest signal available here: continued
                    // presence in this list, not a real activity timestamp.
                    // Background sessions report a real status/state — use it
                    // when present (Observed) instead of assuming (Inferred).
                    let has_confirmed_activity = e.status.as_deref() == Some("busy") || e.state.as_deref() == Some("working");
                    ev.state = Some(StationState::Working);
                    ev.fidelity = if has_confirmed_activity { Fidelity::Observed } else { Fidelity::Inferred };
                    ev.label = Some(crate::redact::redact_field(&raw_label));
                    ev.detail = Some("local".to_string());
                    ev.metrics = Metrics::default();
                    ev.ttl_secs = Some(60);
                    events.push(ev);
                }
                for old_id in self.known_ids.difference(&seen).cloned().collect::<Vec<_>>() {
                    self.known_ids.remove(&old_id);
                    let mut gone = empty_event(now, self.name(), EventKind::SessionGone, EntityRef::new(EntityType::Session, old_id.clone()));
                    gone.session_id = Some(old_id);
                    events.push(gone);
                }
                self.health.record_success(now, CapabilitySet::from_iter([CAP_LOCAL_SESSIONS]));
            }
            None => {
                self.health.record_failure("`claude agents --json` failed, was missing, or returned unparseable output");
            }
        }

        events
    }

    fn health(&self) -> &ObserverHealth {
        &self.health
    }
}

/// Plain `git` shell-out, exactly as §8's data architecture always planned.
/// Read-only: `rev-parse`, `status --porcelain`, `log -1` — nothing mutates
/// the repository.
pub struct GitObserver {
    repo_path: PathBuf,
    health: ObserverHealth,
}

impl GitObserver {
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self { repo_path: repo_path.into(), health: ObserverHealth::new("git") }
    }

    fn run(&self, args: &[&str]) -> Option<String> {
        let out = Command::new("git")
            .arg("-C")
            .arg(&self.repo_path)
            .args(args)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
}

impl Observer for GitObserver {
    fn name(&self) -> &str {
        "git"
    }

    fn poll(&mut self, now: DateTime<Utc>) -> Vec<Event> {
        let mut events = Vec::new();
        let branch = self.run(&["rev-parse", "--abbrev-ref", "HEAD"]);
        let status = self.run(&["status", "--porcelain"]);

        match (&branch, &status) {
            (Some(branch), Some(status)) => {
                let dirty = !status.is_empty();
                let last_commit = self.run(&["log", "-1", "--format=%h %cI %s"]).unwrap_or_else(|| "(no commits)".into());
                let repo_id = self.repo_path.display().to_string();
                let label = format!("branch={branch} dirty={dirty} last_commit=\"{last_commit}\"");

                let mut ev = empty_event(now, self.name(), EventKind::WorktreeChanged, EntityRef::new(EntityType::Check, repo_id));
                ev.label = Some(crate::redact::redact_field(&label));
                ev.fidelity = Fidelity::Observed;
                events.push(ev);
                self.health.record_success(now, CapabilitySet::from_iter([CAP_GIT]));
            }
            _ => {
                self.health.record_failure("git status/rev-parse failed — not a git repo, or git unavailable");
            }
        }

        events
    }

    fn health(&self) -> &ObserverHealth {
        &self.health
    }
}
