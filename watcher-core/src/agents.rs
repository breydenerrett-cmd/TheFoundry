//! Main-side ingest of signed agent bundles (PHASE4_MULTIMACHINE_DESIGN.md,
//! M-01). Verifies authenticity — HMAC-SHA256 signature, a known `key_id`,
//! monotonically-increasing `seq`, and bounded clock skew — before trusting
//! anything inside a bundle. A rejected bundle is a visible per-agent
//! degradation (surfaced via `machines()` -> the `MACHINES` render
//! section), never a silent drop — the same "absence is UNKNOWN, never
//! healthy" rule (§16) the rest of this crate already applies to observers.

use crate::health::{CapabilitySet, ObserverHealth};
use crate::observer::Observer;
use crate::reducer::{MachineRecord, MachineStatus};
use crate::schema::Event;
use crate::sign;
use crate::transport::{AgentBundle, Receiver, SignedBundle};
use chrono::{DateTime, Utc};
use std::collections::BTreeMap;

/// Bundles whose `sent_at` is more than this far from "now" (either
/// direction) are rejected — a stale replay dressed up with a fresh `seq`,
/// or simply a wrong clock on the agent machine, must not be trusted.
pub const DEFAULT_CLOCK_SKEW_SECS: i64 = 5 * 60;

/// Default TTL: an agent that hasn't had a bundle successfully verified
/// within this many seconds renders Unreachable. Its sessions are NOT
/// force-expired here — they age out honestly through the ordinary session
/// TTL path (`StateStore::apply_ttls`) once this observer simply stops
/// emitting fresh events for them, exactly like any other observer going
/// quiet.
pub const DEFAULT_AGENT_TTL_SECS: i64 = 120;

#[derive(Debug, Clone, Default)]
struct AgentState {
    last_heard_at: Option<DateTime<Utc>>,
    last_seq: u64,
    capabilities: Vec<String>,
    /// Reason the MOST RECENT bundle from this agent (if any) was rejected.
    /// Cleared on the next successfully-verified bundle.
    last_error: Option<String>,
}

/// Drains a `Receiver`, verifies each envelope, and turns accepted bundles
/// into ordinary `Event`s the reducer already knows how to fold in — the
/// events themselves carry no special agent-shaped fields; only `machines()`
/// (consulted separately by main.rs) reports per-agent reachability.
pub struct AgentIngestObserver {
    receiver: Box<dyn Receiver>,
    keyring: BTreeMap<String, Vec<u8>>,
    ttl_secs: i64,
    clock_skew_secs: i64,
    agents: BTreeMap<String, AgentState>,
    health: ObserverHealth,
}

impl AgentIngestObserver {
    pub fn new(
        receiver: Box<dyn Receiver>,
        keyring: BTreeMap<String, Vec<u8>>,
        ttl_secs: i64,
    ) -> Self {
        Self {
            receiver,
            keyring,
            ttl_secs,
            clock_skew_secs: DEFAULT_CLOCK_SKEW_SECS,
            agents: BTreeMap::new(),
            health: ObserverHealth::new("agents"),
        }
    }

    /// Best-effort peek at `agent_id` inside `bundle_json`, purely so a
    /// rejection can still be labeled with the right agent row — this value
    /// is NEVER treated as verified; the signature check above every call
    /// site of this happens (or is skipped because it already failed) is
    /// what actually establishes trust.
    fn peek_agent_id(bundle_json: &str) -> Option<String> {
        let v: serde_json::Value = serde_json::from_str(bundle_json).ok()?;
        v.get("agent_id")?.as_str().map(|s| s.to_string())
    }

    fn reject(&mut self, agent_id_hint: &str, reason: String) {
        let entry = self.agents.entry(agent_id_hint.to_string()).or_default();
        entry.last_error = Some(reason);
    }

    /// Verifies one envelope end to end. Returns the bundle's events only if
    /// every check passes; otherwise records the rejection reason against
    /// the best-known agent id and returns nothing.
    fn verify_and_apply(&mut self, signed: &SignedBundle, now: DateTime<Utc>) -> Vec<Event> {
        let hint =
            Self::peek_agent_id(&signed.bundle_json).unwrap_or_else(|| "unknown".to_string());

        let Some(secret) = self.keyring.get(&signed.key_id) else {
            self.reject(&hint, format!("unknown key_id '{}'", signed.key_id));
            return Vec::new();
        };
        if !sign::verify(secret, signed.bundle_json.as_bytes(), &signed.sig_hex) {
            self.reject(&hint, "bad signature".to_string());
            return Vec::new();
        }
        let bundle: AgentBundle = match serde_json::from_str(&signed.bundle_json) {
            Ok(b) => b,
            Err(e) => {
                self.reject(&hint, format!("malformed bundle: {e}"));
                return Vec::new();
            }
        };
        let skew = (now - bundle.sent_at).num_seconds().abs();
        if skew > self.clock_skew_secs {
            self.reject(
                &bundle.agent_id,
                format!("sent_at skew {skew}s exceeds {}s", self.clock_skew_secs),
            );
            return Vec::new();
        }
        let prior_seq = self
            .agents
            .get(&bundle.agent_id)
            .map(|a| a.last_seq)
            .unwrap_or(0);
        if prior_seq > 0 && bundle.seq <= prior_seq {
            self.reject(
                &bundle.agent_id,
                format!("replayed seq {} (last seen {prior_seq})", bundle.seq),
            );
            return Vec::new();
        }

        let caps: Vec<String> = bundle
            .health
            .iter()
            .flat_map(|h| h.capabilities.0.iter().cloned())
            .collect();
        let entry = self.agents.entry(bundle.agent_id.clone()).or_default();
        entry.last_heard_at = Some(now);
        entry.last_seq = bundle.seq;
        entry.capabilities = caps;
        entry.last_error = None;

        bundle.events
    }

    /// Snapshot of every agent this observer has ever heard from (accepted
    /// or rejected), with reachability computed against `now`. Feeds the
    /// `MACHINES` render section via `StateStore::set_machines`.
    pub fn machines(&self, now: DateTime<Utc>) -> Vec<MachineRecord> {
        self.agents
            .iter()
            .map(|(agent_id, state)| {
                let past_ttl = match state.last_heard_at {
                    Some(t) => (now - t).num_seconds() > self.ttl_secs,
                    None => true,
                };
                let unreachable = past_ttl || state.last_error.is_some();
                let reason = if let Some(err) = &state.last_error {
                    Some(err.clone())
                } else if past_ttl {
                    Some("agent unreachable".to_string())
                } else {
                    None
                };
                MachineRecord {
                    agent_id: agent_id.clone(),
                    status: if unreachable {
                        MachineStatus::Unreachable
                    } else {
                        MachineStatus::Reachable
                    },
                    last_heard_at: state.last_heard_at,
                    reason,
                    capabilities: state.capabilities.clone(),
                }
            })
            .collect()
    }
}

impl Observer for AgentIngestObserver {
    fn name(&self) -> &str {
        "agents"
    }

    fn poll(&mut self, now: DateTime<Utc>) -> Vec<Event> {
        let bundles = self.receiver.drain();
        let mut events = Vec::new();
        for signed in &bundles {
            events.extend(self.verify_and_apply(signed, now));
        }
        // This observer's own health is about whether the transport itself
        // was readable at all — per-agent authenticity is a separate axis,
        // reported through `machines()` / the MACHINES section instead.
        self.health
            .record_success(now, CapabilitySet::from_iter(["agent_ingest"]));
        events
    }

    fn health(&self) -> &ObserverHealth {
        &self.health
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{FileTransportPublisher, FileTransportReceiver, Publisher};
    use chrono::Duration;

    fn bundle(agent_id: &str, seq: u64, sent_at: DateTime<Utc>) -> AgentBundle {
        AgentBundle {
            agent_id: agent_id.to_string(),
            seq,
            sent_at,
            events: Vec::new(),
            health: Vec::new(),
        }
    }

    fn sign_bundle(b: &AgentBundle, secret: &[u8], key_id: &str) -> SignedBundle {
        let bundle_json = serde_json::to_string(b).unwrap();
        let sig_hex = sign::sign_hex(secret, bundle_json.as_bytes());
        SignedBundle {
            bundle_json,
            sig_hex,
            key_id: key_id.to_string(),
        }
    }

    #[test]
    fn unknown_key_id_is_rejected() {
        let now = Utc::now();
        let dir = tempfile::tempdir().unwrap();
        let mut pub_ = FileTransportPublisher::new(dir.path(), "pc").unwrap();
        pub_.publish(sign_bundle(&bundle("pc", 1, now), b"secret", "nope"))
            .unwrap();

        let mut keyring = BTreeMap::new();
        keyring.insert("k1".to_string(), b"secret".to_vec());
        let mut ingest = AgentIngestObserver::new(
            Box::new(FileTransportReceiver::new(dir.path())),
            keyring,
            120,
        );
        let events = ingest.poll(now);
        assert!(events.is_empty());
        let rows = ingest.machines(now);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, MachineStatus::Unreachable);
        assert!(rows[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("unknown key_id"));
    }

    #[test]
    fn replayed_seq_is_rejected() {
        let now = Utc::now();
        let dir = tempfile::tempdir().unwrap();
        let mut pub_ = FileTransportPublisher::new(dir.path(), "pc").unwrap();
        pub_.publish(sign_bundle(&bundle("pc", 5, now), b"secret", "k1"))
            .unwrap();

        let mut keyring = BTreeMap::new();
        keyring.insert("k1".to_string(), b"secret".to_vec());
        let mut ingest = AgentIngestObserver::new(
            Box::new(FileTransportReceiver::new(dir.path())),
            keyring,
            120,
        );
        ingest.poll(now);
        assert_eq!(ingest.machines(now)[0].status, MachineStatus::Reachable);

        // Same seq again, later — must be rejected as a replay.
        pub_.publish(sign_bundle(
            &bundle("pc", 5, now + Duration::seconds(1)),
            b"secret",
            "k1",
        ))
        .unwrap();
        ingest.poll(now + Duration::seconds(2));
        let rows = ingest.machines(now + Duration::seconds(2));
        assert_eq!(rows[0].status, MachineStatus::Unreachable);
        assert!(rows[0].reason.as_deref().unwrap().contains("replayed"));
    }

    #[test]
    fn skewed_clock_is_rejected() {
        let now = Utc::now();
        let dir = tempfile::tempdir().unwrap();
        let mut pub_ = FileTransportPublisher::new(dir.path(), "pc").unwrap();
        pub_.publish(sign_bundle(
            &bundle("pc", 1, now - Duration::minutes(20)),
            b"secret",
            "k1",
        ))
        .unwrap();

        let mut keyring = BTreeMap::new();
        keyring.insert("k1".to_string(), b"secret".to_vec());
        let mut ingest = AgentIngestObserver::new(
            Box::new(FileTransportReceiver::new(dir.path())),
            keyring,
            120,
        );
        ingest.poll(now);
        let rows = ingest.machines(now);
        assert_eq!(rows[0].status, MachineStatus::Unreachable);
        assert!(rows[0].reason.as_deref().unwrap().contains("skew"));
    }
}
