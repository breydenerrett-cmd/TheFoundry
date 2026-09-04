//! THE FOUNDRY — Phase 1-3 CLI: watcher core + live text renderer.
//!
//! Usage:
//!   foundry [--feed-dir DIR] [--audit] [--watch SECS] [--log-dir DIR]
//!
//! Single-shot by default (poll once, render once, exit) — pass --watch N to
//! poll every N seconds until Ctrl-C, which is closer to how the real
//! always-on watcher will run.

use chrono::Utc;
use foundry_core::eventlog::EventLog;
use foundry_core::observer::{Observer, RemoteClaudeObserver, SyntheticCanary};
use foundry_core::reducer::StateStore;
use foundry_core::render::{render_audit, render_floor};
use std::path::PathBuf;
use std::time::Duration;

struct Args {
    feed_dir: PathBuf,
    log_dir: PathBuf,
    audit: bool,
    watch_secs: Option<u64>,
}

fn parse_args() -> Args {
    let mut feed_dir = PathBuf::from("live-feed");
    let mut log_dir = PathBuf::from("eventlog");
    let mut audit = false;
    let mut watch_secs = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--feed-dir" => feed_dir = PathBuf::from(args.next().expect("--feed-dir needs a value")),
            "--log-dir" => log_dir = PathBuf::from(args.next().expect("--log-dir needs a value")),
            "--audit" => audit = true,
            "--watch" => {
                let secs: u64 = args.next().expect("--watch needs a value").parse().expect("--watch value must be a number of seconds");
                watch_secs = Some(secs);
            }
            other => eprintln!("warning: unrecognized argument '{other}', ignoring"),
        }
    }
    Args { feed_dir, log_dir, audit, watch_secs }
}

fn main() {
    let args = parse_args();

    let mut remote = RemoteClaudeObserver::new(&args.feed_dir);
    let mut canary = SyntheticCanary::new();
    let mut store = StateStore::new();
    let mut log = EventLog::new(&args.log_dir, 50_000, 30).expect("failed to open event log directory");

    loop {
        let now = Utc::now();

        let remote_events = remote.poll(now);
        let canary_events = canary.poll(now);

        let mut all_events = remote_events;
        all_events.extend(canary_events);

        store.apply_events(&all_events, now);
        store.apply_observer_health(remote.health(), now);
        store.apply_observer_health(canary.health(), now);

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

        match args.watch_secs {
            Some(secs) => std::thread::sleep(Duration::from_secs(secs)),
            None => break,
        }
    }
}
