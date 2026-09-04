//! Pluggable delivery for agent bundles (PHASE4_MULTIMACHINE_DESIGN.md,
//! M-01). `Publisher`/`Receiver` are the ONLY contract the agent binary and
//! main ingest (`agents.rs`) are allowed to depend on — the wire mechanism
//! is deliberately swappable, exactly like `RemoteClaudeObserver`'s adapter
//! boundary. `FileTransport{Publisher,Receiver}` is the one concrete
//! implementation built so far: a shared directory of `<agent_id>.jsonl`
//! files, one signed envelope per line, read back by tracked byte offset so
//! repeated `drain()` calls only return newly-appended lines.
//!
//! INTENDED NEXT IMPLEMENTATION (not built): an authenticated HTTP push,
//! agent -> main's LAN address, per the design doc's recommendation — same
//! `Publisher`/`Receiver` contract, just a network call instead of a file
//! append/read. Nothing in `agents.rs` or the schema should need to change
//! when that lands.

use crate::health::ObserverHealth;
use crate::schema::Event;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, Cursor, Write};
use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, String>;

/// One publish from a single agent process. `events` already carry their
/// rewritten `source` (`"<observer>@<agent_id>"`) and have already been
/// through redaction — nothing downstream should need to touch either
/// again.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentBundle {
    pub agent_id: String,
    pub seq: u64,
    pub sent_at: DateTime<Utc>,
    pub events: Vec<Event>,
    pub health: Vec<ObserverHealth>,
}

/// The authenticated envelope actually carried by a `Publisher`/`Receiver`.
/// `bundle_json` is the exact canonical byte string the signature covers —
/// verification must run against these bytes, never a re-serialization of
/// the parsed `AgentBundle` (which could differ in field order/whitespace).
/// `key_id` is a short, non-secret label identifying which shared secret
/// signed this envelope — never the secret itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedBundle {
    pub bundle_json: String,
    pub sig_hex: String,
    pub key_id: String,
}

pub trait Publisher {
    fn publish(&mut self, bundle: SignedBundle) -> Result<()>;
}

pub trait Receiver {
    /// Returns every envelope newly available since the last call. Must
    /// never re-return an envelope already handed back — callers rely on
    /// `drain()` being a one-shot consuming read of "what's new".
    fn drain(&mut self) -> Vec<SignedBundle>;
}

/// Agent-side `Publisher`: appends one JSON line per publish to
/// `<dir>/<agent_id>.jsonl`. Never truncates or rewrites existing lines.
pub struct FileTransportPublisher {
    path: PathBuf,
}

impl FileTransportPublisher {
    pub fn new(dir: impl Into<PathBuf>, agent_id: &str) -> std::io::Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        Ok(Self {
            path: dir.join(format!("{agent_id}.jsonl")),
        })
    }
}

impl Publisher for FileTransportPublisher {
    fn publish(&mut self, bundle: SignedBundle) -> Result<()> {
        let line = serde_json::to_string(&bundle).map_err(|e| e.to_string())?;
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| e.to_string())?;
        writeln!(f, "{line}").map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Main-side `Receiver`: watches every `*.jsonl` file in `dir` (one per
/// agent), tracking a byte offset per file so repeated `drain()` calls only
/// return lines appended since the last read. A file that shrank (e.g.
/// replaced out from under us) is treated as reset-to-start rather than
/// causing an out-of-bounds read.
pub struct FileTransportReceiver {
    dir: PathBuf,
    offsets: BTreeMap<PathBuf, u64>,
}

impl FileTransportReceiver {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            offsets: BTreeMap::new(),
        }
    }
}

impl Receiver for FileTransportReceiver {
    fn drain(&mut self) -> Vec<SignedBundle> {
        let mut out = Vec::new();
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return out;
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|e| e == "jsonl").unwrap_or(false))
            .collect();
        paths.sort();

        for path in paths {
            let Ok(bytes) = fs::read(&path) else {
                continue;
            };
            let offset = *self.offsets.get(&path).unwrap_or(&0);
            let offset = if (offset as usize) > bytes.len() {
                0 // file shrank/was replaced — read it from the start again
            } else {
                offset
            };
            let new_bytes = &bytes[offset as usize..];
            for line in Cursor::new(new_bytes)
                .lines()
                .map_while(std::io::Result::ok)
            {
                if line.trim().is_empty() {
                    continue;
                }
                if let Ok(sb) = serde_json::from_str::<SignedBundle>(&line) {
                    out.push(sb);
                }
            }
            self.offsets.insert(path, bytes.len() as u64);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sb(n: u32) -> SignedBundle {
        SignedBundle {
            bundle_json: format!("{{\"n\":{n}}}"),
            sig_hex: "deadbeef".into(),
            key_id: "k1".into(),
        }
    }

    #[test]
    fn receiver_only_returns_new_lines_across_calls() {
        let dir = tempfile::tempdir().unwrap();
        let mut publisher = FileTransportPublisher::new(dir.path(), "pc").unwrap();
        let mut receiver = FileTransportReceiver::new(dir.path());

        publisher.publish(sb(1)).unwrap();
        let first = receiver.drain();
        assert_eq!(first.len(), 1);

        // No new publishes — a second drain must be empty, not re-deliver.
        assert!(receiver.drain().is_empty());

        publisher.publish(sb(2)).unwrap();
        let second = receiver.drain();
        assert_eq!(second.len(), 1);
        assert!(second[0].bundle_json.contains('2'));
    }

    #[test]
    fn multiple_agent_files_are_all_drained() {
        let dir = tempfile::tempdir().unwrap();
        let mut pc = FileTransportPublisher::new(dir.path(), "pc").unwrap();
        let mut mac = FileTransportPublisher::new(dir.path(), "mac").unwrap();
        pc.publish(sb(1)).unwrap();
        mac.publish(sb(2)).unwrap();

        let mut receiver = FileTransportReceiver::new(dir.path());
        let all = receiver.drain();
        assert_eq!(all.len(), 2);
    }
}
