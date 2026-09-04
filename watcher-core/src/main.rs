//! THE FOUNDRY — Phase 1-3.5 CLI: watcher core + live text renderer.
//!
//! Usage:
//!   foundry [--feed-dir DIR] [--git-dir DIR] [--no-remote] [--audit] [--watch SECS] [--log-dir DIR] [--bay-map PATH] [--no-restore]
//!
//! S-01 persistence: `--log-dir` is also where `snapshot.json` lives. On
//! startup (unless `--no-restore`) a prior snapshot is loaded and any
//! logged events after its `last_seq` are replayed on top of it — but every
//! restored record renders STALE/UNKNOWN "(restored)" and the pipeline
//! stays UNVERIFIED until a fresh event from a real observer lands in THIS
//! process (see `foundry_core::persist`). The snapshot is re-saved at the
//! end of every poll cycle.
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
use foundry_core::agents::AgentIngestObserver;
use foundry_core::bay::BayMap;
use foundry_core::eventlog::EventLog;
use foundry_core::heartbeat::HeartbeatObserver;
use foundry_core::local::{GitObserver, LocalClaudeObserver};
use foundry_core::observer::{Observer, RemoteClaudeObserver, SyntheticCanary};
use foundry_core::persist::{self, LoadOutcome};
use foundry_core::reducer::StateStore;
use foundry_core::render::{render_audit, render_floor};
use foundry_core::transport::FileTransportReceiver;
use std::collections::BTreeMap;
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
    bay_map_path: PathBuf,
    /// S-01: skip loading `<log-dir>/snapshot.json` at startup even if it
    /// exists — always start from a clean, fully-fresh `StateStore`.
    no_restore: bool,
    /// Phase 4D M-01: directory a `FileTransportReceiver` drains
    /// `<agent_id>.jsonl` envelopes from. `None` means no agent ingest at
    /// all — the MACHINES section is simply omitted, same as any other
    /// unconfigured observer.
    agents_dir: Option<PathBuf>,
    agent_ttl_secs: i64,
    /// `"id=path,id2=path2"` — one or more key_id -> secret-file mappings.
    agent_keys_arg: Option<String>,
    /// Paired with `FOUNDRY_AGENT_KEY` in the environment for the
    /// single-key case: `--agent-key-id NAME` + `FOUNDRY_AGENT_KEY=...`.
    agent_key_id: Option<String>,
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
    let mut bay_map_path = PathBuf::from("foundry.bays.toml");
    let mut no_restore = false;
    let mut agents_dir = None;
    let mut agent_ttl_secs = foundry_core::agents::DEFAULT_AGENT_TTL_SECS;
    let mut agent_keys_arg = None;
    let mut agent_key_id = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--feed-dir" => {
                feed_dir = PathBuf::from(args.next().expect("--feed-dir needs a value"))
            }
            "--git-dir" => git_dir = PathBuf::from(args.next().expect("--git-dir needs a value")),
            "--heartbeat-dir" => {
                heartbeat_dir = Some(PathBuf::from(
                    args.next().expect("--heartbeat-dir needs a value"),
                ))
            }
            "--heartbeat-label" => {
                heartbeat_label = args.next().expect("--heartbeat-label needs a value")
            }
            "--log-dir" => log_dir = PathBuf::from(args.next().expect("--log-dir needs a value")),
            "--audit" => audit = true,
            "--no-remote" => no_remote = true,
            "--no-restore" => no_restore = true,
            "--bay-map" => {
                bay_map_path = PathBuf::from(args.next().expect("--bay-map needs a value"))
            }
            "--watch" => {
                let secs: u64 = args
                    .next()
                    .expect("--watch needs a value")
                    .parse()
                    .expect("--watch value must be a number of seconds");
                watch_secs = Some(secs);
            }
            "--agents-dir" => {
                agents_dir = Some(PathBuf::from(
                    args.next().expect("--agents-dir needs a value"),
                ))
            }
            "--agent-ttl" => {
                agent_ttl_secs = args
                    .next()
                    .expect("--agent-ttl needs a value")
                    .parse()
                    .expect("--agent-ttl value must be a number of seconds")
            }
            "--agent-keys" => {
                agent_keys_arg = Some(args.next().expect("--agent-keys needs a value"))
            }
            "--agent-key-id" => {
                agent_key_id = Some(args.next().expect("--agent-key-id needs a value"))
            }
            other => eprintln!("warning: unrecognized argument '{other}', ignoring"),
        }
    }
    Args {
        feed_dir,
        git_dir,
        heartbeat_dir,
        heartbeat_label,
        log_dir,
        audit,
        watch_secs,
        no_remote,
        bay_map_path,
        no_restore,
        agents_dir,
        agent_ttl_secs,
        agent_keys_arg,
        agent_key_id,
    }
}

/// Builds the key_id -> shared-secret map for verifying agent bundles, from
/// `--agent-keys id=path,...` and/or `--agent-key-id NAME` paired with the
/// `FOUNDRY_AGENT_KEY` environment variable. The secret is read ONLY from a
/// file or that env var — never accepted as a bare CLI argument value, and
/// never logged.
/// F-8: `--agent-keys` accepts two shapes per comma-separated entry:
///   - `id=path` (original, unrestricted — key_id `id` may sign for any
///     agent_id, exactly as before);
///   - `agent_id=key_id=path` (bound — key_id `key_id` may ONLY sign bundles
///     that claim `agent_id`; a mismatch is rejected, visibly, by
///     `AgentIngestObserver`).
///
/// Returns the keyring plus the key_id -> agent_id binding map — key_ids
/// never bound via the 2-part form are left unrestricted.
fn build_agent_keyring(
    agent_keys_arg: &Option<String>,
    agent_key_id: &Option<String>,
) -> (BTreeMap<String, Vec<u8>>, BTreeMap<String, String>) {
    let mut map = BTreeMap::new();
    let mut bindings = BTreeMap::new();
    if let Some(spec) = agent_keys_arg {
        for pair in spec.split(',') {
            let pair = pair.trim();
            if pair.is_empty() {
                continue;
            }
            let parts: Vec<&str> = pair.splitn(3, '=').collect();
            match parts.as_slice() {
                [agent_id, key_id, path] => match std::fs::read_to_string(path) {
                    Ok(secret) => {
                        map.insert(key_id.to_string(), secret.trim().as_bytes().to_vec());
                        bindings.insert(key_id.to_string(), agent_id.to_string());
                    }
                    Err(e) => eprintln!(
                        "warning: could not read agent key file '{path}' for key_id '{key_id}' (agent '{agent_id}'): {e}"
                    ),
                },
                [id, path] => match std::fs::read_to_string(path) {
                    Ok(secret) => {
                        map.insert(id.to_string(), secret.trim().as_bytes().to_vec());
                    }
                    Err(e) => eprintln!(
                        "warning: could not read agent key file '{path}' for key_id '{id}': {e}"
                    ),
                },
                _ => eprintln!(
                    "warning: malformed --agent-keys entry '{pair}', expected id=path or agent_id=key_id=path"
                ),
            }
        }
    }
    if let Some(id) = agent_key_id {
        match std::env::var("FOUNDRY_AGENT_KEY") {
            Ok(secret) => {
                map.insert(id.clone(), secret.into_bytes());
            }
            Err(_) => eprintln!(
                "warning: --agent-key-id given but FOUNDRY_AGENT_KEY is not set — that key_id will reject everything"
            ),
        }
    }
    (map, bindings)
}

fn main() {
    let args = parse_args();

    let mut remote = (!args.no_remote).then(|| RemoteClaudeObserver::new(&args.feed_dir));
    let mut local_claude = LocalClaudeObserver::new();
    let mut git = GitObserver::new(&args.git_dir);
    let mut heartbeat = args
        .heartbeat_dir
        .as_ref()
        .map(|d| HeartbeatObserver::new(d, args.heartbeat_label.clone()));
    let mut canary = SyntheticCanary::new();
    let mut agent_ingest = args.agents_dir.as_ref().map(|dir| {
        let (keyring, bindings) = build_agent_keyring(&args.agent_keys_arg, &args.agent_key_id);
        AgentIngestObserver::new(
            Box::new(FileTransportReceiver::new(dir)),
            keyring,
            args.agent_ttl_secs,
        )
        .with_key_bindings(bindings)
    });
    let mut log =
        EventLog::new(&args.log_dir, 50_000, 30).expect("failed to open event log directory");

    // S-01 restart recovery: load the last snapshot (if any), replay
    // anything logged after it, and mark every restored record honestly
    // stale/unverified rather than trusting it as live.
    let mut store = if args.no_restore {
        StateStore::new()
    } else {
        match persist::load_snapshot(&args.log_dir) {
            LoadOutcome::Missing => StateStore::new(),
            LoadOutcome::Corrupted(msg) => {
                eprintln!(
                    "warning: snapshot at {} is corrupted/unreadable ({msg}) — starting fresh, nothing restored",
                    args.log_dir.display()
                );
                StateStore::new()
            }
            LoadOutcome::Loaded(snapshot) => {
                let mut restored_store = snapshot.store;
                persist::mark_restored(&mut restored_store, snapshot.saved_at);
                match log.events_since(snapshot.last_seq) {
                    Ok(replay) => {
                        let replay_events: Vec<_> = replay.into_iter().map(|pe| pe.event).collect();
                        if !replay_events.is_empty() {
                            restored_store.apply_events(&replay_events, Utc::now());
                        }
                    }
                    Err(e) => {
                        eprintln!(
                            "warning: failed to read event log for replay after restore: {e}"
                        );
                    }
                }
                eprintln!(
                    "restored snapshot from {} (saved {}, last_seq {}) — all sessions/routines/checks render STALE/UNKNOWN until re-observed this run",
                    args.log_dir.display(),
                    snapshot.saved_at,
                    snapshot.last_seq,
                );
                restored_store
            }
        }
    };

    // F-11: seed the agent replay guard from whatever watermark survived in
    // the snapshot — otherwise a restart reopens the seq replay window for
    // every previously-known agent, not just seq 0 (F-3).
    if let Some(ai) = &mut agent_ingest {
        ai.restore_seq_watermarks(&store.agent_seq_watermarks);
    }

    let bay_map = match BayMap::load(&args.bay_map_path) {
        Ok(map) => map,
        Err(_) => {
            eprintln!(
                "note: no bay map at {} — everything will be UNRESOLVED",
                args.bay_map_path.display()
            );
            BayMap::new()
        }
    };

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
        if let Some(ai) = &mut agent_ingest {
            all_events.extend(ai.poll(now));
        }

        store.apply_events(&all_events, now);
        if let Some(remote) = &remote {
            store.apply_observer_health(
                remote.health(),
                now,
                Some(foundry_core::observer::CAP_SESSIONS),
                Some(foundry_core::observer::CAP_ROUTINES),
            );
        }
        store.apply_observer_health(
            local_claude.health(),
            now,
            Some(foundry_core::local::CAP_LOCAL_SESSIONS),
            None,
        );
        store.apply_observer_health(git.health(), now, None, None);
        if let Some(hb) = &heartbeat {
            store.apply_observer_health(hb.health(), now, None, None);
        }
        store.apply_observer_health(canary.health(), now, None, None);
        if let Some(ai) = &agent_ingest {
            store.apply_observer_health(ai.health(), now, None, None);
            store.set_machines(ai.machines(now));
            store.set_agent_seq_watermarks(ai.seq_watermarks());
        }

        if let Err(e) = log.append(&all_events) {
            eprintln!("warning: event log write failed: {e}");
        }

        // S-01: persist the state store at the end of every poll cycle
        // (single-shot runs get exactly one save; `--watch` saves on every
        // cycle too, which trivially satisfies "every N cycles" for any N).
        if let Err(e) = persist::save_snapshot(&args.log_dir, &store, log.last_seq(), now) {
            eprintln!("warning: snapshot write failed: {e}");
        }

        let rendered = if args.audit {
            render_audit(&store, now, &args.feed_dir, &bay_map)
        } else {
            render_floor(&store, now, &bay_map)
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
