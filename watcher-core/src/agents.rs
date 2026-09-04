//! Main-side ingest of signed agent bundles (PHASE4_MULTIMACHINE_DESIGN.md,
//! M-01). Verifies authenticity — HMAC-SHA256 signature, a known `key_id`,
//! monotonically-increasing `seq`, and bounded clock skew — before trusting
//! anything inside a bundle. A rejected bundle is a visible per-agent
//! degradation (surfaced via `machines()` -> the `MACHINES` render
//! section), never a silent drop — the same "absence is UNKNOWN, never
//! healthy" rule (§16) the rest of this crate already applies to observers.

use crate::health::{CapabilitySet, ObserverHealth};
use crate::observer::Observer;
use crate::redact::redact_field;
use crate::reducer::{MachineRecord, MachineStatus};
use crate::schema::Event;
use crate::sign;
use crate::transport::{AgentBundle, Receiver, SignedBundle};
use chrono::{DateTime, Utc};
use std::collections::BTreeMap;

/// F-9: the bucket a rejection is attributed to BEFORE the bundle's
/// signature has verified — an unauthenticated writer can put whatever it
/// wants in the `agent_id` field of a bundle it never signed correctly, so
/// keying the MACHINES row on that claimed-but-unverified value would let
/// it fabricate an arbitrary machine's row (rename a dead/nonexistent
/// machine, or paint over a real one under a name it doesn't own). Every
/// pre-signature-verification rejection (unknown key_id, bad signature,
/// unparseable bundle) is attributed here instead; only once the signature
/// has actually verified is `bundle.agent_id` trustworthy enough to key a
/// row by.
pub const UNVERIFIED_BUCKET: &str = "unverified";

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
    /// F-3: `None` means "no bundle from this agent has ever been accepted"
    /// — distinct from `Some(0)`, a genuine, already-accepted `seq == 0`.
    /// The old `u64` (implicitly starting at 0) could not tell those apart,
    /// so a bundle carrying `seq == 0` never tripped the replay guard
    /// (`prior_seq > 0 && seq <= prior_seq`) and could be re-published
    /// forever to resurrect a dead machine.
    last_seq: Option<u64>,
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
    /// F-8: which agent_id a given key_id is authorized to sign for. Empty
    /// (the default, and every pre-existing caller of `new`) means no
    /// binding is enforced — backward compatible with configurations that
    /// only ever hand out one key_id per deployment. When a key_id IS bound
    /// here, a signature that verifies under it but claims a DIFFERENT
    /// agent_id is rejected as a visible degradation rather than silently
    /// accepted: a shared secret does not, by itself, prove which machine is
    /// allowed to publish under which name.
    key_bindings: BTreeMap<String, String>,
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
            key_bindings: BTreeMap::new(),
            ttl_secs,
            clock_skew_secs: DEFAULT_CLOCK_SKEW_SECS,
            agents: BTreeMap::new(),
            health: ObserverHealth::new("agents"),
        }
    }

    /// F-8: binds each `key_id` to the single `agent_id` it may sign for.
    /// Builder-style so existing call sites (unbound — any agent_id may use
    /// any key_id it holds the secret for) don't need to change.
    pub fn with_key_bindings(mut self, bindings: BTreeMap<String, String>) -> Self {
        self.key_bindings = bindings;
        self
    }

    fn reject(&mut self, agent_id_hint: &str, reason: String) {
        let entry = self.agents.entry(agent_id_hint.to_string()).or_default();
        // §15/F-10: rejection reasons are rendered/snapshotted verbatim
        // (MACHINES section, `--audit`) — never trust a writer's own
        // key_id/agent_id/JSON-error text without scrubbing it first.
        entry.last_error = Some(redact_field(&reason));
    }

    /// F-11: exports the current `agent_id -> last accepted seq` watermark
    /// for every agent this observer has ever accepted a bundle from, so the
    /// caller can persist it (see `StateStore::agent_seq_watermarks`) and
    /// hand it back via `restore_seq_watermarks` after a restart — otherwise
    /// a restart reopens the whole replay window (F-3) for every agent, not
    /// just seq 0.
    pub fn seq_watermarks(&self) -> BTreeMap<String, u64> {
        self.agents
            .iter()
            .filter_map(|(id, s)| s.last_seq.map(|seq| (id.clone(), seq)))
            .collect()
    }

    /// F-11: seeds this observer's replay guard from a persisted watermark
    /// (loaded from a snapshot) BEFORE the first `poll()` — never lowers an
    /// already-known watermark, only raises it.
    pub fn restore_seq_watermarks(&mut self, watermarks: &BTreeMap<String, u64>) {
        for (id, seq) in watermarks {
            let entry = self.agents.entry(id.clone()).or_default();
            if entry.last_seq.is_none_or(|cur| *seq > cur) {
                entry.last_seq = Some(*seq);
            }
        }
    }

    /// Verifies one envelope end to end. Returns the bundle's events only if
    /// every check passes; otherwise records the rejection reason against
    /// the best-known agent id and returns nothing.
    fn verify_and_apply(&mut self, signed: &SignedBundle, now: DateTime<Utc>) -> Vec<Event> {
        // F-9: everything up to a verified signature is attacker-controlled
        // input — `signed.key_id` and anything inside `bundle_json`
        // (including its claimed `agent_id`) are NOT trustworthy yet, so
        // rejections here must never key a MACHINES row off them.
        let Some(secret) = self.keyring.get(&signed.key_id) else {
            self.reject(
                UNVERIFIED_BUCKET,
                format!("unknown key_id '{}'", redact_field(&signed.key_id)),
            );
            return Vec::new();
        };
        if !sign::verify(secret, signed.bundle_json.as_bytes(), &signed.sig_hex) {
            self.reject(UNVERIFIED_BUCKET, "bad signature".to_string());
            return Vec::new();
        }
        let bundle: AgentBundle = match serde_json::from_str(&signed.bundle_json) {
            Ok(b) => b,
            Err(e) => {
                self.reject(UNVERIFIED_BUCKET, format!("malformed bundle: {e}"));
                return Vec::new();
            }
        };

        // F-8: the signature is now known-good for `signed.key_id`, but that
        // alone doesn't say WHICH agent_id it's allowed to publish as — a
        // key_id bound to one machine must not be able to sign bundles that
        // claim to be a different one.
        if let Some(expected) = self.key_bindings.get(&signed.key_id) {
            if expected != &bundle.agent_id {
                self.reject(
                    &bundle.agent_id,
                    format!(
                        "key_id '{}' is not authorized for agent_id '{}'",
                        redact_field(&signed.key_id),
                        redact_field(&bundle.agent_id)
                    ),
                );
                return Vec::new();
            }
        }

        let skew = (now - bundle.sent_at).num_seconds().abs();
        if skew > self.clock_skew_secs {
            self.reject(
                &bundle.agent_id,
                format!("sent_at skew {skew}s exceeds {}s", self.clock_skew_secs),
            );
            return Vec::new();
        }
        let prior_seq = self.agents.get(&bundle.agent_id).and_then(|a| a.last_seq);
        if let Some(prior) = prior_seq {
            if bundle.seq <= prior {
                self.reject(
                    &bundle.agent_id,
                    format!("replayed seq {} (last seen {prior})", bundle.seq),
                );
                return Vec::new();
            }
        }

        let caps: Vec<String> = bundle
            .health
            .iter()
            .flat_map(|h| h.capabilities.0.iter().cloned())
            .collect();
        let entry = self.agents.entry(bundle.agent_id.clone()).or_default();
        // F-11: the agent's own report of when it sent this bundle, not when
        // WE happened to poll and drain it — polling is not observation.
        entry.last_heard_at = Some(bundle.sent_at);
        entry.last_seq = Some(bundle.seq);
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
        // F-7: this observer's own health is about whether the transport
        // itself was readable at all — per-agent authenticity is a separate
        // axis, reported through `machines()` / the MACHINES section
        // instead. An unreadable transport (missing/misconfigured
        // `--agents-dir`) must record_failure, not the unconditional
        // record_success this used to be — otherwise a directory nothing
        // can ever be received from still painted a green HEALTHY row.
        let bundles = match self.receiver.drain() {
            Ok(b) => b,
            Err(e) => {
                self.health.record_failure(e);
                return Vec::new();
            }
        };
        let mut events = Vec::new();
        for signed in &bundles {
            events.extend(self.verify_and_apply(signed, now));
        }
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
