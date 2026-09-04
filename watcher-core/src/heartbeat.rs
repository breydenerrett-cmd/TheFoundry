//! Phase 4B — `.foundry/events.jsonl` heartbeat contract (§8 "heartbeat
//! adapters"). Observed systems emit their own one-line-per-event JSONL;
//! Foundry only ever reads it. Never writes to any observed repo.
//!
//! Contract (one JSON object per line, append-only):
//! ```json
//! {"component":"forward_capture","event":"end","status":"ok","ts":"2026-09-04T07:15:03Z","artifact":"3 files changed","error":null}
//! ```
//! - `component`: stable short id (e.g. "forward_capture", "daily_loop",
//!   "monitor_remote", "test_runner").
//! - `event`: "start" | "end" | "status" | "escalate".
//! - `status`: "ok" | "degraded" | "down" | "escalate".
//! - `ts`: RFC3339.
//! - `artifact` (optional): short descriptive string only — a file count, a
//!   commit sha, a URL. NEVER raw output, prompts, or file contents.
//! - `error` (optional): a short one-line description. NEVER a stack trace,
//!   secret, or transcript excerpt.
//!
//! This observer is READ-ONLY and defensive: it redacts every free-text
//! field again anyway (never trust the writer alone — same principle as
//! §15's defense-in-depth rule already applied to `--audit`).

use crate::health::{CapabilitySet, ObserverHealth};
use crate::observer::Observer;
use crate::schema::{EntityRef, EntityType, Event, EventKind, Fidelity, Metrics};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const CAP_HEARTBEAT: &str = "project_heartbeat";

#[derive(Debug, Deserialize)]
struct RawHeartbeatLine {
    component: String,
    event: String,
    status: String,
    ts: String,
    #[serde(default)]
    artifact: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

/// Reads `<repo>/.foundry/events.jsonl`, keeping only the LATEST line per
/// `component` each poll (the file is append-only and can grow; Foundry
/// never truncates or writes to it).
pub struct HeartbeatObserver {
    path: PathBuf,
    label: String,
    health: ObserverHealth,
}

impl HeartbeatObserver {
    pub fn new(repo_path: impl Into<PathBuf>, label: impl Into<String>) -> Self {
        let repo_path = repo_path.into();
        Self {
            path: repo_path.join(".foundry").join("events.jsonl"),
            label: label.into(),
            health: ObserverHealth::new("heartbeat"),
        }
    }
}

impl Observer for HeartbeatObserver {
    fn name(&self) -> &str {
        "heartbeat"
    }

    fn poll(&mut self, now: DateTime<Utc>) -> Vec<Event> {
        let mut events = Vec::new();
        let Ok(contents) = std::fs::read_to_string(&self.path) else {
            // Missing file is a legitimate, common state (not yet
            // instrumented, or nothing has run yet) — degrade quietly,
            // don't treat as a crash.
            self.health.record_failure("`.foundry/events.jsonl` not present or unreadable");
            return events;
        };

        let mut latest: BTreeMap<String, RawHeartbeatLine> = BTreeMap::new();
        let mut parsed_any = false;
        for line in contents.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(rec) = serde_json::from_str::<RawHeartbeatLine>(line) {
                parsed_any = true;
                latest.insert(rec.component.clone(), rec);
            }
            // Malformed lines are silently skipped, per-line, exactly like
            // RemoteClaudeObserver's per-record tolerance — one bad line
            // must not lose the rest of the file.
        }

        for (component, rec) in latest {
            let status_note = match rec.status.as_str() {
                "ok" => "ok",
                "degraded" => "DEGRADED",
                "down" => "DOWN",
                "escalate" => "ESCALATE",
                _ => "unknown-status",
            };
            let mut label = format!("[{}] {} {} — {}", self.label, component, rec.event, status_note);
            if let Some(a) = &rec.artifact {
                label.push_str(&format!(" ({a})"));
            }
            if let Some(e) = &rec.error {
                label.push_str(&format!(" error=\"{e}\""));
            }
            let label = crate::redact::redact_field(&label);

            let ts = DateTime::parse_from_rfc3339(&rec.ts).ok().map(|d| d.with_timezone(&Utc));

            // Reuses the Check-record path (§1a EQUIPMENT, no reasoning loop)
            // that GitObserver already established via WorktreeChanged.
            let repo_hint = self.path.parent().and_then(|p| p.parent()).map(|p| p.display().to_string());
            let mut entity = EntityRef::new(EntityType::Check, format!("{}/{}", self.label, component));
            if let Some(hint) = repo_hint {
                entity = entity.with_parent(hint);
            }
            let mut ev = crate::local::empty_event(now, self.name(), EventKind::WorktreeChanged, entity);
            ev.label = Some(label);
            ev.fidelity = Fidelity::Observed;
            ev.metrics = Metrics { elapsed_ms: ts.map(|t| (now - t).num_milliseconds().max(0)), ..Default::default() };
            events.push(ev);
        }

        if parsed_any || contents.trim().is_empty() {
            self.health.record_success(now, CapabilitySet::from_iter([CAP_HEARTBEAT]));
        } else {
            self.health.record_failure("`.foundry/events.jsonl` exists but no line parsed");
        }
        events
    }

    fn health(&self) -> &ObserverHealth {
        &self.health
    }
}
