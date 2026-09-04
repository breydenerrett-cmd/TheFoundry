//! Live text renderer (§17a truth gate) + `--audit` ground-truth mode.
//!
//! This is deliberately plain text, not the Pixi floor — Phase 3's job is to
//! prove the numbers are true, not to look good. Color is a convenience for
//! reading a terminal, nothing here depends on it being present.

use crate::health::ObserverStatus;
use crate::reducer::StateStore;
use crate::schema::{Fidelity, StationState};
use chrono::{DateTime, Utc};
use std::path::Path;

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";

fn state_color(s: StationState) -> &'static str {
    match s {
        StationState::Working => "\x1b[32m",           // green
        StationState::Thinking => "\x1b[34m",           // blue
        StationState::Specialist => "\x1b[35m",         // violet
        StationState::WaitingOnAgent | StationState::WaitingOnSystem | StationState::Blocked => "\x1b[33m", // amber, dim
        StationState::BreyRequired => "\x1b[1;33m",     // amber, bold — strongest
        StationState::Failed => "\x1b[1;31m",           // red, bold
        StationState::Hung => "\x1b[31m",                // red-orange
        StationState::Idle => "\x1b[90m",                // gray
        StationState::Completed => "\x1b[37m",           // white->gray
        StationState::StaleUnknown => "\x1b[2;37m",      // dimmed/hatched proxy
    }
}

fn state_label(s: StationState) -> &'static str {
    match s {
        StationState::Working => "WORKING",
        StationState::Thinking => "THINKING",
        StationState::Specialist => "SPECIALIST",
        StationState::WaitingOnAgent => "WAITING_ON_AGENT",
        StationState::WaitingOnSystem => "WAITING_ON_SYSTEM",
        StationState::Blocked => "BLOCKED",
        StationState::BreyRequired => "BREY_REQUIRED",
        StationState::Failed => "FAILED",
        StationState::Hung => "HUNG",
        StationState::Idle => "IDLE",
        StationState::Completed => "COMPLETED",
        StationState::StaleUnknown => "STALE/UNKNOWN",
    }
}

fn fidelity_tag(f: Fidelity) -> &'static str {
    match f {
        Fidelity::Observed => "observed",
        Fidelity::Inferred => "inferred",
        Fidelity::Unknown => "unknown",
    }
}

fn fmt_age(secs: i64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// Renders the estate-wide floor as plain text (§4 L1/L3 content, no visuals).
pub fn render_floor(store: &StateStore, now: DateTime<Utc>) -> String {
    let mut out = String::new();
    let pipeline_ok = store.pipeline_verified(now, 300);

    // Marquee (§2 point 6 / §5a).
    let mut counts = std::collections::BTreeMap::<&str, u32>::new();
    for rec in store.sessions.values() {
        if rec.gone {
            continue;
        }
        *counts.entry(state_label(rec.displayed_state.value)).or_insert(0) += 1;
    }
    let overdue_routines = store.routines.values().filter(|r| r.overdue).count();
    // Soonest-due enabled routine, by actual next_run_at — not iteration order,
    // and never a disabled routine (it has no meaningful "next").
    let next_routine = store
        .routines
        .values()
        .filter(|r| r.enabled)
        .filter_map(|r| r.next_run_at.map(|t| (t, &r.name)))
        .min_by_key(|(t, _)| *t)
        .map(|(t, name)| format!("{name} ({})", t.format("%H:%M UTC")))
        .unwrap_or_else(|| "—".into());

    out.push_str(&format!("{BOLD}══════════════════════════════════════════════════════════════{RESET}\n"));
    if pipeline_ok {
        out.push_str(&format!("{BOLD} THE FOUNDRY — LIVE (pipeline verified){RESET}\n"));
    } else {
        out.push_str(&format!("\x1b[1;31m THE FOUNDRY — UNVERIFIED (no recent canary heartbeat — DO NOT TRUST){RESET}\n"));
    }
    let counts_str: Vec<String> = counts.iter().map(|(k, v)| format!("{v} {k}")).collect();
    out.push_str(&format!(
        " {} · {} routine(s) overdue · next: {} · LAST OUTPUT: n/a (no output observer wired in Phase 1-3)\n",
        if counts_str.is_empty() { "no sessions observed".to_string() } else { counts_str.join(" · ") },
        overdue_routines,
        next_routine,
    ));
    if let Some(age) = store.last_sync_age_secs(now) {
        out.push_str(&format!(" last-sync age: {}\n", fmt_age(age)));
    } else {
        out.push_str(" last-sync age: never synced\n");
    }
    out.push_str(&format!("{BOLD}══════════════════════════════════════════════════════════════{RESET}\n\n"));

    // Sessions (§4 L1/L3 content — no bay/room grouping yet, that's Phase 5).
    out.push_str(&format!("{BOLD}SESSIONS{RESET} ({} observed)\n", store.sessions.values().filter(|r| !r.gone).count()));
    for rec in store.sessions.values() {
        if rec.gone {
            continue;
        }
        let color = state_color(rec.displayed_state.value);
        let label = state_label(rec.displayed_state.value);
        let fidelity = fidelity_tag(rec.displayed_state.fidelity);
        let warn = if rec.stall_warning && rec.displayed_state.value == StationState::Working {
            " [stall-warning]"
        } else {
            ""
        };
        let elapsed = (now - rec.last_activity_at).num_seconds().max(0);
        let env = rec.session_kind.as_ref().map(|f| f.value.as_str()).unwrap_or("unknown-kind");
        let model = rec.model.as_ref().map(|f| f.value.as_str()).unwrap_or("?");
        let model_current = rec.model_current.as_ref().map(|f| f.value.as_str());
        let model_served = rec.model_last_served.as_ref().map(|f| f.value.as_str());
        let mut model_str = model.to_string();
        if let Some(mc) = model_current {
            if mc != model {
                model_str.push_str(&format!(" (current: {mc})"));
            }
        }
        if let Some(ms) = model_served {
            if Some(ms) != model_current && ms != model {
                model_str.push_str(&format!(" (last-served: {ms})"));
            }
        }
        let task = rec.label.as_ref().map(|f| f.value.as_str()).unwrap_or("(no task summary)");

        out.push_str(&format!(
            "  {color}[{label:>16}]{RESET}{warn} {DIM}({fidelity}){RESET} {} [{env}] model={model_str} elapsed={} task=\"{task}\"\n",
            rec.id,
            fmt_age(elapsed),
        ));
    }
    out.push('\n');

    // Routines.
    out.push_str(&format!("{BOLD}ROUTINES{RESET} ({} observed)\n", store.routines.len()));
    for r in store.routines.values() {
        let tag = if !r.enabled {
            "\x1b[90m[DISABLED]\x1b[0m"
        } else if r.overdue {
            "\x1b[1;33m[OVERDUE]\x1b[0m"
        } else {
            "\x1b[32m[ON SCHEDULE]\x1b[0m"
        };
        let bound = r.bound_session_id.as_deref().unwrap_or("(fresh-session routine)");
        let next = r.next_run_at.map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string()).unwrap_or_else(|| "unknown".into());
        out.push_str(&format!("  {tag} {} — next: {next} — bound: {bound}\n", r.name));
        if let Some(p) = &r.prompt_redacted {
            out.push_str(&format!("      prompt (redacted): {DIM}{p}{RESET}\n"));
        }
    }
    out.push('\n');

    // Observer health.
    out.push_str(&format!("{BOLD}OBSERVERS{RESET}\n"));
    for h in store.observer_health.values() {
        let status_color = match h.status {
            ObserverStatus::Healthy => "\x1b[32m",
            ObserverStatus::Degraded => "\x1b[33m",
            ObserverStatus::Down => "\x1b[1;31m",
            ObserverStatus::Unverified => "\x1b[90m",
        };
        let age = h.last_sync_age_secs(now).map(fmt_age).unwrap_or_else(|| "never".into());
        let caps: Vec<&str> = h.capabilities.0.iter().map(|s| s.as_str()).collect();
        out.push_str(&format!(
            "  {status_color}[{:>10}]{RESET} {} — capabilities: [{}] — last-sync: {} — failures: {}\n",
            format!("{:?}", h.status).to_uppercase(),
            h.name,
            caps.join(", "),
            age,
            h.consecutive_failures,
        ));
        if let Some(err) = &h.last_error {
            out.push_str(&format!("      last error: {DIM}{err}{RESET}\n"));
        }
    }

    out
}

/// `--audit`: state-store view side by side with the raw feed files it was
/// built from, so a human can spot-check the mapping directly (§18).
pub fn render_audit(store: &StateStore, now: DateTime<Utc>, feed_dir: &Path) -> String {
    let mut out = String::new();
    out.push_str(&render_floor(store, now));
    out.push_str(&format!("\n{BOLD}── AUDIT: raw feed vs normalized state ──{RESET}\n"));
    for filename in ["list_sessions.json", "list_triggers.json"] {
        let path = feed_dir.join(filename);
        out.push_str(&format!("\n{DIM}raw file: {}{RESET}\n", path.display()));
        match std::fs::read_to_string(&path) {
            Ok(raw) => {
                let preview: String = raw.chars().take(2000).collect();
                out.push_str(&preview);
                if raw.chars().count() > 2000 {
                    out.push_str(&format!("\n{DIM}...[truncated for audit display]{RESET}"));
                }
                out.push('\n');
            }
            Err(e) => out.push_str(&format!("{DIM}(unreadable: {e}){RESET}\n")),
        }
    }
    out.push_str(&format!(
        "\n{DIM}normalized session count: {} | routine count: {} | observers: {}{RESET}\n",
        store.sessions.values().filter(|r| !r.gone).count(),
        store.routines.len(),
        store.observer_health.len(),
    ));
    out
}
