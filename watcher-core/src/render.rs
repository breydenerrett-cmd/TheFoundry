//! Live text renderer (§17a truth gate) + `--audit` ground-truth mode.
//!
//! This is deliberately plain text, not the Pixi floor — Phase 3's job is to
//! prove the numbers are true, not to look good. Color is a convenience for
//! reading a terminal, nothing here depends on it being present.

use crate::health::ObserverStatus;
use crate::local::CAP_LOCAL_SESSIONS;
use crate::observer::CAP_SESSIONS;
use crate::reducer::{StateStore, GONE_FADE_SECS};
use crate::schema::{Fidelity, StationState};
use chrono::{DateTime, Utc};
use std::path::Path;

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";

fn state_color(s: StationState) -> &'static str {
    match s {
        StationState::Working => "\x1b[32m",    // green
        StationState::Thinking => "\x1b[34m",   // blue
        StationState::Specialist => "\x1b[35m", // violet
        StationState::WaitingOnAgent | StationState::WaitingOnSystem | StationState::Blocked => {
            "\x1b[33m"
        } // amber, dim
        StationState::BreyRequired => "\x1b[1;33m", // amber, bold — strongest
        StationState::Failed => "\x1b[1;31m",   // red, bold
        StationState::Hung => "\x1b[31m",       // red-orange
        StationState::Idle => "\x1b[90m",       // gray
        StationState::Completed => "\x1b[37m",  // white->gray
        StationState::StaleUnknown => "\x1b[2;37m", // dimmed/hatched proxy
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
pub fn render_floor(
    store: &StateStore,
    now: DateTime<Utc>,
    bay_map: &crate::bay::BayMap,
) -> String {
    let mut out = String::new();
    let pipeline_ok = store.pipeline_verified(now, 300);

    // Marquee (§2 point 6 / §5a). Shared with `export.rs` via
    // `StateStore::session_state_counts`/`overdue_routines_count`/
    // `next_routine` so the live text and the JSON export can never
    // silently disagree about these numbers.
    let mut counts = std::collections::BTreeMap::<&str, u32>::new();
    for (state, n) in store.session_state_counts() {
        *counts.entry(state_label(state)).or_insert(0) += n;
    }
    let overdue_routines = store.overdue_routines_count();
    let next_routine = store
        .next_routine()
        .map(|(t, name)| format!("{name} ({})", t.format("%H:%M UTC")))
        .unwrap_or_else(|| "—".into());

    out.push_str(&format!(
        "{BOLD}══════════════════════════════════════════════════════════════{RESET}\n"
    ));
    if pipeline_ok {
        out.push_str(&format!(
            "{BOLD} THE FOUNDRY — LIVE (pipeline verified){RESET}\n"
        ));
    } else if let Some(restored_at) = store.pipeline.restored_at {
        // S-01: a snapshot was loaded this process and no fresh (non-canary)
        // observer event has landed yet — the floor must not read as LIVE
        // just because the canary happens to be ticking.
        let age = (now - restored_at).num_seconds().max(0);
        out.push_str(&format!(
            "\x1b[1;31m THE FOUNDRY — UNVERIFIED (restored snapshot, age {} — awaiting fresh observations){RESET}\n",
            fmt_age(age)
        ));
    } else if store.any_real_observer_down() {
        let down_names: Vec<&str> = store
            .observer_health
            .values()
            .filter(|h| h.name != "synthetic_canary" && matches!(h.status, ObserverStatus::Down))
            .map(|h| h.name.as_str())
            .collect();
        out.push_str(&format!(
            "\x1b[1;31m THE FOUNDRY — UNVERIFIED (observer(s) DOWN: {} — DO NOT TRUST){RESET}\n",
            down_names.join(", ")
        ));
    } else {
        out.push_str(&format!("\x1b[1;31m THE FOUNDRY — UNVERIFIED (no recent canary heartbeat — DO NOT TRUST){RESET}\n"));
    }
    // Phase 4C: Remote/cloud estate gets its OWN explicit freshness line,
    // separate from the local-observer-driven pipeline banner above. A
    // standalone process cannot poll this continuously (§ access-bridge
    // report — no zero-cost read API exists), so it is refreshed manually
    // on-demand; old data must never silently read as current.
    const REMOTE_STALE_SECS: i64 = 30 * 60;
    match store.observer_health.get("remote_claude") {
        Some(h) => match h.last_sync_age_secs(now) {
            Some(age) if age <= REMOTE_STALE_SECS => {
                out.push_str(&format!(
                    " {DIM}REMOTE ESTATE: live (last sync {}){RESET}\n",
                    fmt_age(age)
                ));
            }
            Some(age) => {
                out.push_str(&format!(" \x1b[1;33mREMOTE ESTATE — DEGRADED{RESET} {DIM}(last sync {} ago — run the manual refresh; see PHASE3_5_ACCESS_BRIDGE.md §6){RESET}\n", fmt_age(age)));
            }
            None => {
                out.push_str(&format!(" \x1b[1;33mREMOTE ESTATE — DEGRADED{RESET} {DIM}(never synced this run){RESET}\n"));
            }
        },
        None => {
            out.push_str(&format!(" {DIM}REMOTE ESTATE: not running (--no-remote) — local observers only, not faked{RESET}\n"));
        }
    }
    // §5a: a confirmed zero and "we couldn't check" must never look the
    // same. `sessions_confirmed` is true only if some observer THIS poll
    // actually reported the sessions capability — only then is an empty
    // count a real fact rather than a blind spot (adversarial finding #3).
    let sessions_confirmed = store
        .observer_health
        .values()
        .any(|h| h.capabilities.has(CAP_SESSIONS) || h.capabilities.has(CAP_LOCAL_SESSIONS));
    let counts_str: Vec<String> = counts.iter().map(|(k, v)| format!("{v} {k}")).collect();
    let sessions_summary = if counts_str.is_empty() {
        if sessions_confirmed {
            "no sessions observed".to_string()
        } else {
            "SESSIONS: UNKNOWN (sessions capability unavailable — not a confirmed zero)".to_string()
        }
    } else {
        counts_str.join(" · ")
    };
    let last_output = match store.last_output_at() {
        Some(t) => format!(
            "LAST OUTPUT: {} ago",
            fmt_age((now - t).num_seconds().max(0))
        ),
        None => "LAST OUTPUT: n/a (no heartbeat-sourced output observed yet)".to_string(),
    };
    out.push_str(&format!(
        " {sessions_summary} · {} routine(s) overdue · next: {} · {last_output}\n",
        overdue_routines, next_routine,
    ));
    if let Some(age) = store.last_sync_age_secs(now) {
        out.push_str(&format!(" last-sync age: {}\n", fmt_age(age)));
    } else {
        out.push_str(" last-sync age: never synced\n");
    }
    out.push_str(&format!(
        "{BOLD}══════════════════════════════════════════════════════════════{RESET}\n\n"
    ));

    // Sessions (§4 L1/L3 content), grouped by §9/Phase 4E bay resolution.
    let session_count_label = if sessions_confirmed {
        format!(
            "({} observed)",
            store.sessions.values().filter(|r| !r.gone).count()
        )
    } else {
        "(UNKNOWN — sessions capability unavailable)".to_string()
    };
    out.push_str(&format!("{BOLD}SESSIONS{RESET} {session_count_label}\n"));
    // §16: a vanished session gets a 60s "fading" grace state, then folds
    // down as ENDED — it must never just disappear from the floor without a
    // trace (adversarial finding #5). `ended_count` tallies everything past
    // the fade window so the footer still acknowledges it happened.
    let mut ended_count = 0u32;
    let mut by_bay: std::collections::BTreeMap<String, Vec<_>> = std::collections::BTreeMap::new();
    for rec in store.sessions.values() {
        let resolution = bay_map.resolve(rec.repo_hint.as_deref(), &[]);
        by_bay.entry(resolution.bay).or_default().push(rec);
    }
    // UNRESOLVED is a distinct display group and always renders last, after
    // the six real bays (which sort alphabetically among themselves).
    let unresolved_recs = by_bay.remove(crate::bay::UNRESOLVED);
    let ordered_bays = by_bay
        .into_iter()
        .chain(unresolved_recs.map(|recs| (crate::bay::UNRESOLVED.to_string(), recs)));
    for (bay, recs) in ordered_bays {
        if bay == crate::bay::UNRESOLVED {
            out.push_str(&format!(
                "  {BOLD}▸ {bay} (no repo/tag match — not guessed){RESET}\n"
            ));
        } else {
            out.push_str(&format!("  {BOLD}▸ {bay}{RESET}\n"));
        }
        for rec in recs {
            if rec.gone {
                let faded_secs = rec
                    .gone_at
                    .map(|t| (now - t).num_seconds())
                    .unwrap_or(i64::MAX);
                if faded_secs > GONE_FADE_SECS {
                    ended_count += 1;
                    continue;
                }
            }
            let color = if rec.gone {
                "\x1b[2;37m"
            } else {
                state_color(rec.displayed_state.value)
            };
            let label: &str = if rec.gone {
                "FADING/ENDED"
            } else {
                state_label(rec.displayed_state.value)
            };
            let fidelity = fidelity_tag(rec.displayed_state.fidelity);
            let warn = if rec.stall_warning && rec.displayed_state.value == StationState::Working {
                " [stall-warning]"
            } else {
                ""
            };
            // S-01: a record carried over from a loaded snapshot, not a live
            // observation in this process — its displayed_state is already
            // forced to StaleUnknown by `persist::mark_restored`, but this
            // tag is what tells a human WHY a session that used to be
            // WORKING now reads STALE/UNKNOWN.
            let restored_tag = if rec.restored { " (restored)" } else { "" };
            let elapsed = (now - rec.last_activity_at).num_seconds().max(0);
            let env = rec
                .session_kind
                .as_ref()
                .map(|f| f.value.as_str())
                .unwrap_or("unknown-kind");
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
            let task = rec
                .label
                .as_ref()
                .map(|f| f.value.as_str())
                .unwrap_or("(no task summary)");

            out.push_str(&format!(
            "  {color}[{label:>16}]{RESET}{warn}{restored_tag} {DIM}({fidelity}){RESET} {} [{env}] model={model_str} elapsed={} task=\"{task}\"\n",
            rec.id,
            fmt_age(elapsed),
        ));
        }
    }
    if ended_count > 0 {
        out.push_str(&format!(
            "  {DIM}({ended_count} session(s) ended this run, past the fade window){RESET}\n"
        ));
    }
    out.push('\n');

    // Routines.
    out.push_str(&format!(
        "{BOLD}ROUTINES{RESET} ({} observed)\n",
        store.routines.len()
    ));
    for r in store.routines.values() {
        // `stale` wins over everything else — a routine whose observer just
        // lost the routines capability must not still show a confident
        // ON SCHEDULE/OVERDUE/DISABLED verdict computed from a now-unverified
        // snapshot (adversarial finding #4).
        let tag = if r.stale {
            "\x1b[2;37m[STALE]\x1b[0m"
        } else if !r.enabled {
            "\x1b[90m[DISABLED]\x1b[0m"
        } else if r.overdue {
            "\x1b[1;33m[OVERDUE]\x1b[0m"
        } else {
            "\x1b[32m[ON SCHEDULE]\x1b[0m"
        };
        let bound = r
            .bound_session_id
            .as_deref()
            .unwrap_or("(fresh-session routine)");
        let next = r
            .next_run_at
            .map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string())
            .unwrap_or_else(|| "unknown".into());
        let staleness_note = if r.stale {
            " (last confirmed values shown — not verified this poll)"
        } else {
            ""
        };
        let restored_note = if r.restored { " (restored)" } else { "" };
        out.push_str(&format!(
            "  {tag} {} — next: {next} — bound: {bound}{staleness_note}{restored_note}\n",
            r.name
        ));
        if let Some(p) = &r.prompt_redacted {
            out.push_str(&format!("      prompt (redacted): {DIM}{p}{RESET}\n"));
        }
    }
    out.push('\n');

    // Observer health.
    // Checks (§1a EQUIPMENT — deterministic facts, e.g. git state).
    if !store.checks.is_empty() {
        out.push_str(&format!(
            "{BOLD}CHECKS{RESET} ({} observed)\n",
            store.checks.len()
        ));
        for c in store.checks.values() {
            let restored_note = if c.restored { " (restored)" } else { "" };
            out.push_str(&format!(
                "  {DIM}[{}]{RESET} {} — {}{restored_note}\n",
                c.source, c.id, c.label.value
            ));
        }
        out.push('\n');
    }

    out.push_str(&format!("{BOLD}OBSERVERS{RESET}\n"));
    for h in store.observer_health.values() {
        let status_color = match h.status {
            ObserverStatus::Healthy => "\x1b[32m",
            ObserverStatus::Degraded => "\x1b[33m",
            ObserverStatus::Down => "\x1b[1;31m",
            ObserverStatus::Unverified => "\x1b[90m",
        };
        let age = h
            .last_sync_age_secs(now)
            .map(fmt_age)
            .unwrap_or_else(|| "never".into());
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

    // Phase 4D M-01: one row per agent this process has ever heard from
    // (accepted or rejected) — omitted entirely when no `--agents-dir` was
    // configured, exactly like the CHECKS section above.
    if !store.machines.is_empty() {
        out.push_str(&format!("\n{BOLD}MACHINES{RESET}\n"));
        for m in store.machines.values() {
            let (status_color, status_label) = match m.status {
                crate::reducer::MachineStatus::Reachable => ("\x1b[32m", "REACHABLE"),
                crate::reducer::MachineStatus::Unreachable => ("\x1b[1;31m", "UNREACHABLE"),
            };
            let age = m
                .last_heard_at
                .map(|t| fmt_age((now - t).num_seconds().max(0)))
                .unwrap_or_else(|| "never".into());
            let caps = m.capabilities.join(", ");
            out.push_str(&format!(
                "  {status_color}[{status_label:>11}]{RESET} {} — last-heard: {} — capabilities: [{}]\n",
                m.agent_id, age, caps,
            ));
            if let Some(r) = &m.reason {
                out.push_str(&format!("      reason: {DIM}{r}{RESET}\n"));
            }
        }
    }

    out
}

/// `--audit`: state-store view side by side with the raw feed files it was
/// built from, so a human can spot-check the mapping directly (§18).
pub fn render_audit(
    store: &StateStore,
    now: DateTime<Utc>,
    feed_dir: &Path,
    bay_map: &crate::bay::BayMap,
) -> String {
    let mut out = String::new();
    out.push_str(&render_floor(store, now, bay_map));
    out.push_str(&format!(
        "\n{BOLD}── AUDIT: raw feed vs normalized state ──{RESET}\n"
    ));
    for filename in ["list_sessions.json", "list_triggers.json"] {
        let path = feed_dir.join(filename);
        out.push_str(&format!("\n{DIM}raw file: {}{RESET}\n", path.display()));
        match std::fs::read_to_string(&path) {
            Ok(raw) => {
                // Defense in depth (adversarial finding #6): the feed is
                // *expected* to already be redacted by the bridge before it
                // ever lands here, but --audit is a diagnostic screen that
                // people will actually look at — it must not be the one
                // place that trusts that on faith.
                let raw = crate::redact::scrub_secrets(&raw);
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
