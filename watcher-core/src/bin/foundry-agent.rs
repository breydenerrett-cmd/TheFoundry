//! THE FOUNDRY AGENT — Phase 4D M-01: a thin per-machine process that runs
//! the SAME local, zero-model-token observers already built for the main
//! binary (`local_claude`, `git` x N, optional `.foundry` heartbeat) and
//! publishes a signed, authenticated bundle each poll, via a
//! `transport::Publisher` (today: `FileTransport`, a shared directory).
//!
//! Usage:
//!   FOUNDRY_AGENT_KEY=<secret> foundry-agent --agent-id pc --transport-dir DIR \
//!       --key-id k1 [--git-dir DIR ...] [--heartbeat-dir DIR] [--once | --watch SECS]
//!
//! The shared secret is read ONLY from `FOUNDRY_AGENT_KEY` or a
//! `--key-file PATH` — never accepted as a bare CLI argument, and never
//! printed or logged by this binary.
//!
//! Every event's `source` is rewritten to `"<observer>@<agent_id>"` before
//! it is signed (e.g. `local_claude@pc`) — the main side's reducer already
//! treats `source` as an arbitrary observer name, so per-machine
//! degradation/staleness/capability tracking works completely unchanged.
//! `label`/`detail` are redacted again here (defense in depth, same
//! principle already applied to `--audit`) since a bundle crosses a real
//! trust boundary once it leaves this process.

use chrono::Utc;
use foundry_core::heartbeat::HeartbeatObserver;
use foundry_core::local::{GitObserver, LocalClaudeObserver};
use foundry_core::observer::Observer;
use foundry_core::redact::redact_field;
use foundry_core::sign;
use foundry_core::transport::{AgentBundle, FileTransportPublisher, Publisher, SignedBundle};
use std::path::PathBuf;
use std::time::Duration;

struct Args {
    agent_id: String,
    transport_dir: PathBuf,
    key_file: Option<PathBuf>,
    key_id: String,
    git_dirs: Vec<PathBuf>,
    heartbeat_dir: Option<PathBuf>,
    heartbeat_label: String,
    watch_secs: Option<u64>,
}

fn parse_args() -> Args {
    let default_agent_id = hostname_fallback();
    let mut agent_id = default_agent_id;
    let mut transport_dir = None;
    let mut key_file = None;
    let mut key_id = "default".to_string();
    let mut git_dirs = Vec::new();
    let mut heartbeat_dir = None;
    let mut heartbeat_label = "SPORTS LAB".to_string();
    let mut watch_secs = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--agent-id" => agent_id = args.next().expect("--agent-id needs a value"),
            "--transport-dir" => {
                transport_dir = Some(PathBuf::from(
                    args.next().expect("--transport-dir needs a value"),
                ))
            }
            "--key-file" => {
                key_file = Some(PathBuf::from(
                    args.next().expect("--key-file needs a value"),
                ))
            }
            "--key-id" => key_id = args.next().expect("--key-id needs a value"),
            "--git-dir" => {
                git_dirs.push(PathBuf::from(args.next().expect("--git-dir needs a value")))
            }
            "--heartbeat-dir" => {
                heartbeat_dir = Some(PathBuf::from(
                    args.next().expect("--heartbeat-dir needs a value"),
                ))
            }
            "--heartbeat-label" => {
                heartbeat_label = args.next().expect("--heartbeat-label needs a value")
            }
            "--once" => {} // default behavior already — accepted for clarity at the call site
            "--watch" => {
                let secs: u64 = args
                    .next()
                    .expect("--watch needs a value")
                    .parse()
                    .expect("--watch value must be a number of seconds");
                watch_secs = Some(secs);
            }
            other => eprintln!("warning: unrecognized argument '{other}', ignoring"),
        }
    }

    Args {
        agent_id,
        transport_dir: transport_dir.expect("--transport-dir is required"),
        key_file,
        key_id,
        git_dirs,
        heartbeat_dir,
        heartbeat_label,
        watch_secs,
    }
}

fn hostname_fallback() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "unknown-machine".to_string())
}

/// Reads the shared signing secret ONLY from `FOUNDRY_AGENT_KEY` or
/// `--key-file` — never from a CLI argument value, and this function never
/// prints or logs the value it returns.
fn load_secret(key_file: &Option<PathBuf>) -> Vec<u8> {
    if let Ok(secret) = std::env::var("FOUNDRY_AGENT_KEY") {
        return secret.trim().as_bytes().to_vec();
    }
    if let Some(path) = key_file {
        let secret = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("could not read --key-file {}: {e}", path.display()));
        return secret.trim().as_bytes().to_vec();
    }
    panic!("no signing key: set FOUNDRY_AGENT_KEY or pass --key-file PATH");
}

/// Defense-in-depth redaction pass over the bundle's free-text fields
/// before it is serialized and signed — `local_claude`/`git` already redact
/// `label` at the observer boundary, but a bundle is about to cross an
/// actual network/filesystem trust boundary to another machine, so this
/// re-scrubs rather than trusting that on faith (same principle as
/// `render::render_audit`'s defense-in-depth scrub).
fn redact_event(ev: &mut foundry_core::schema::Event) {
    if let Some(l) = &ev.label {
        ev.label = Some(redact_field(l));
    }
    if let Some(d) = &ev.detail {
        ev.detail = Some(redact_field(d));
    }
}

fn main() {
    let args = parse_args();
    let secret = load_secret(&args.key_file);

    let mut local_claude = LocalClaudeObserver::new();
    let mut gits: Vec<GitObserver> = args.git_dirs.iter().map(GitObserver::new).collect();
    let mut heartbeat = args
        .heartbeat_dir
        .as_ref()
        .map(|d| HeartbeatObserver::new(d, args.heartbeat_label.clone()));

    let mut publisher = FileTransportPublisher::new(&args.transport_dir, &args.agent_id)
        .expect("failed to open transport directory");

    let mut seq: u64 = 1;
    loop {
        let now = Utc::now();
        let mut events = Vec::new();
        let mut health = Vec::new();

        for ev in local_claude.poll(now) {
            events.push(ev);
        }
        health.push(local_claude.health().clone());

        for git in &mut gits {
            for ev in git.poll(now) {
                events.push(ev);
            }
            health.push(git.health().clone());
        }

        if let Some(hb) = &mut heartbeat {
            for ev in hb.poll(now) {
                events.push(ev);
            }
            health.push(hb.health().clone());
        }

        // Rewrite `source` (and, where present, the session-kind slot in
        // `detail`) to carry this machine's identity, then redact again
        // before this bundle leaves the process.
        for ev in &mut events {
            ev.source = format!("{}@{}", ev.source, args.agent_id);
            if let Some(d) = &ev.detail {
                ev.detail = Some(format!("{d}@{}", args.agent_id));
            }
            redact_event(ev);
        }

        let bundle = AgentBundle {
            agent_id: args.agent_id.clone(),
            seq,
            sent_at: now,
            events,
            health,
        };
        let bundle_json = serde_json::to_string(&bundle).expect("bundle serializes to JSON");
        let sig_hex = sign::sign_hex(&secret, bundle_json.as_bytes());
        let signed = SignedBundle {
            bundle_json,
            sig_hex,
            key_id: args.key_id.clone(),
        };

        match publisher.publish(signed) {
            Ok(()) => eprintln!(
                "foundry-agent[{}]: published seq={seq} ({} event(s))",
                args.agent_id,
                bundle.events.len()
            ),
            Err(e) => eprintln!("foundry-agent[{}]: publish failed: {e}", args.agent_id),
        }
        seq += 1;

        match args.watch_secs {
            Some(secs) => std::thread::sleep(Duration::from_secs(secs)),
            None => break,
        }
    }
}
