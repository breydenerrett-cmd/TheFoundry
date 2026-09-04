//! THE FOUNDRY — Phase 1-3.5 CLI: watcher core + live text renderer.
//!
//! Usage:
//!   foundry [--feed-dir DIR] [--git-dir DIR] [--no-remote] [--audit] [--watch SECS] [--log-dir DIR]
//!
//! Single-shot by default (poll once, render once, exit) — pass --watch N to
//! poll every N seconds until Ctrl-C, which is closer to how the real
//! always-on watcher will run.
//!
//! Phase 3.5 adds two zero-model-token, standalone-capable observers
//! (`local_claude`, `git`) alongside the existing manually-fed
//! `remote_claude` bridge — pass `--no-remote` to run WITHOUT it and prove
//! the local-only degraded mode honestly renders Remote/cloud capability as
//! unavailable rather than faking estate-wide visibility. See
//! PHASE3_5_ACCESS_BRIDGE.md.

use chrono::Utc;
use foundry_core::eventlog::EventLog;
use foundry_core::heartbeat::HeartbeatObserver;
use foundry_core::local::{GitObserver, LocalClaudeObserver};
use foundry_core::observer::{Observer, RemoteClaudeObserver, SyntheticCanary};
use foundry_core::reducer::StateStore;
use foundry_core::render::{render_audit, render_floor};
use std::path::PathBuf;
use std::time::Duration;

struct Args {
    feed_dir: PathBuf,
    git_dir: PathBuf,
    heartbeat_dir: Option<PathBuf>,
    heartbeat_label: String,
    log_dir: PathBuf,
    audit: bool,
    watch_secs: Option<u64>,
    no_remote: bool,
}

fn parse_args() -> Args {
    let mut feed_dir = PathBuf::from("live-feed");
    let mut git_dir = PathBuf::from(".");
    let mut heartbeat_dir = None;
    let mut heartbeat_label = "SPORTS LAB".to_string();
    let mut log_dir = PathBuf::from("eventlog");
    let mut audit = false;
    let mut watch_secs = None;
    let mut no_remote = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--feed-dir" => feed_dir = PathBuf::from(args.next().expect("--feed-dir needs a value")),
            "--git-dir" => git_dir = PathBuf::from(args.next().expect("--git-dir needs a value")),
            "--heartbeat-dir" => heartbeat_dir = Some(PathBuf::from(args.next().expect("--heartbeat-dir needs a value"))),
            "--heartbeat-label" => heartbeat_label = args.next().expect("--heartbeat-label needs a value"),
            "--log-dir" => log_dir = PathBuf::from(args.next().expect("--log-dir needs a value")),
            "--audit" => audit = true,
            "--no-remote" => no_remote = true,
            "--watch" => {
                let secs: u64 = args.next().expect("--watch needs a value").parse().expect("--watch value must be a number of seconds");
                watch_secs = Some(secs);
            }
            other => eprintln!("warning: unrecognized argument '{other}', ignoring"),
        }
    }
    Args { feed_dir, git_dir, heartbeat_dir, heartbeat_label, log_dir, audit, watch_secs, no_remote }
}

fn main() {
    let args = parse_args();

    let mut remote = (!args.no_remote).then(|| RemoteClaudeObserver::new(&args.feed_dir));
    let mut local_claude = LocalClaudeObserver::new();
    let mut git = GitObserver::new(&args.git_dir);
    let mut heartbeat = args.heartbeat_dir.as_ref().map(|d| HeartbeatObserver::new(d, args.heartbeat_label.clone()));
    let mut canary = SyntheticCanary::new();
    let mut store = StateStore::new();
    let mut log = EventLog::new(&args.log_dir, 50_000, 30).expect("failed to open event log directory");

    loop {
        let now = Utc::now();

        let mut all_events = Vec::new();
        if let Some(remote) = &mut remote {
            all_events.extend(remote.poll(now));
        }
        all_events.extend(local_claude.poll(now));
        all_events.extend(git.poll(now));
        if let Some(hb) = &mut heartbeat {
            all_events.extend(hb.poll(now));
        }
        all_events.extend(canary.poll(now));

        store.apply_events(&all_events, now);
        if let Some(remote) = &remote {
            store.apply_observer_health(remote.health(), now, Some(foundry_core::observer::CAP_SESSIONS), Some(foundry_core::observer::CAP_ROUTINES));
        }
        store.apply_observer_health(local_claude.health(), now, Some(foundry_core::local::CAP_LOCAL_SESSIONS), None);
        store.apply_observer_health(git.health(), now, None, None);
        if let Some(hb) = &heartbeat {
            store.apply_observer_health(hb.health(), now, None, None);
        }
        store.apply_observer_health(canary.health(), now, None, None);

        if let Err(e) = log.append(&all_events) {
            eprintln!("warning: event log write failed: {e}");
        }

        let rendered = if args.audit {
            render_audit(&store, now, &args.feed_dir)
        } else {
            render_floor(&store, now)
        };

        // Clear-ish separation between polls when watching, so it reads like
        // a live-refreshing screen rather than an unbroken scroll.
        if args.watch_secs.is_some() {
            println!("\n\n");
        }
        println!("{rendered}");
        if args.no_remote {
            println!("(--no-remote: remote_claude observer not started — Remote/cloud sessions are intentionally absent, not faked)");
        }

        match args.watch_secs {
            Some(secs) => std::thread::sleep(Duration::from_secs(secs)),
            None => break,
        }
    }
}
