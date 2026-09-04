//! Observer health / capability tracking (§7a, §16). An observer declares what
//! it can currently supply; losing a capability degrades visibly, it never
//! makes the reducer quietly treat the missing signal as healthy.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitySet(pub BTreeSet<String>);

impl CapabilitySet {
    pub fn new() -> Self {
        Self(BTreeSet::new())
    }
    #[allow(clippy::should_implement_trait)]
    pub fn from_iter<I: IntoIterator<Item = &'static str>>(iter: I) -> Self {
        Self(iter.into_iter().map(|s| s.to_string()).collect())
    }
    pub fn has(&self, cap: &str) -> bool {
        self.0.contains(cap)
    }
    /// Capabilities present in `before` but missing from `self`.
    pub fn lost_since(&self, before: &CapabilitySet) -> Vec<String> {
        before.0.difference(&self.0).cloned().collect()
    }
    /// Capabilities present in `self` but not in `before`.
    pub fn gained_since(&self, before: &CapabilitySet) -> Vec<String> {
        self.0.difference(&before.0).cloned().collect()
    }
}

impl Default for CapabilitySet {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObserverStatus {
    Healthy,
    Degraded,
    Down,
    /// Never polled yet.
    Unverified,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObserverHealth {
    pub name: String,
    pub status: ObserverStatus,
    /// Capabilities this observer could supply on the MOST RECENT poll —
    /// callers (the reducer) check this per-capability to degrade only the
    /// specific data that's actually missing, not everything the observer
    /// has ever sourced (adversarial finding #3: a Down status was only
    /// reached when ALL capabilities failed, which let a partial loss —
    /// e.g. sessions gone but routines still parsing — hide behind a
    /// HEALTHY badge).
    pub capabilities: CapabilitySet,
    /// The union of every capability this observer has EVER successfully
    /// supplied. Used to detect regression: losing a capability it used to
    /// have is Degraded even though `capabilities` is still non-empty.
    pub known_capabilities: CapabilitySet,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub consecutive_failures: u32,
    /// Whether this observer overrides freshness reporting with an explicit
    /// data-capture timestamp (see `set_data_age`) rather than the poll time
    /// in `last_success_at`. Most observers never call `set_data_age`, so
    /// `last_sync_age_secs` keeps its original poll-time semantics for them.
    #[serde(default)]
    data_age_tracked: bool,
    /// When `data_age_tracked` is true: the timestamp the underlying data was
    /// actually captured/observed, if known. `None` means the observer knows
    /// it is tracking data age but could not determine one (e.g. a snapshot
    /// with no capture timestamp) — freshness must then read as unknown, not
    /// silently fall back to poll time.
    #[serde(default)]
    data_observed_at: Option<DateTime<Utc>>,
}

impl ObserverHealth {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: ObserverStatus::Unverified,
            capabilities: CapabilitySet::new(),
            known_capabilities: CapabilitySet::new(),
            last_success_at: None,
            last_error: None,
            consecutive_failures: 0,
            data_age_tracked: false,
            data_observed_at: None,
        }
    }

    pub fn record_success(&mut self, at: DateTime<Utc>, capabilities: CapabilitySet) {
        self.last_success_at = Some(at);
        self.last_error = None;
        self.consecutive_failures = 0;

        // Regression check: did we lose something we used to be able to
        // supply? A non-empty `capabilities` this poll is not enough to call
        // the observer Healthy if it used to also give us more.
        let regressed = !capabilities.lost_since(&self.known_capabilities).is_empty();
        self.known_capabilities
            .0
            .extend(capabilities.0.iter().cloned());
        self.capabilities = capabilities;

        self.status = if self.capabilities.0.is_empty() {
            ObserverStatus::Down
        } else if regressed {
            ObserverStatus::Degraded
        } else {
            ObserverStatus::Healthy
        };
    }

    pub fn record_failure(&mut self, error: impl Into<String>) {
        self.consecutive_failures += 1;
        // §15: `last_error` is rendered/snapshotted verbatim (the floor and
        // `--audit` both print it) — route it through the same redaction
        // boundary every other free-text field crosses before it can carry
        // a path, email, or accidentally-pasted secret out.
        self.last_error = Some(crate::redact::redact_field(&error.into()));
        // This poll confirmed NOTHING — `capabilities` must reflect that,
        // not silently keep last poll's now-stale value. Leaving the old
        // (non-empty) capability set in place was a real bug: the reducer's
        // per-capability degradation check (`health.capabilities.has(...)`)
        // would see a capability as still present when this exact poll just
        // failed to confirm it at all.
        self.capabilities = CapabilitySet::new();
        self.status = if self.consecutive_failures >= 3 {
            ObserverStatus::Down
        } else {
            ObserverStatus::Degraded
        };
    }

    /// Age of the last successful poll, or None if never succeeded. For an
    /// observer that has called `set_data_age`, this instead reports the age
    /// of the underlying DATA (e.g. when a remote snapshot was captured),
    /// which can be far older than the poll that merely read it off disk —
    /// and is `None` if that data-capture time is tracked but unknown.
    pub fn last_sync_age_secs(&self, now: DateTime<Utc>) -> Option<i64> {
        if self.data_age_tracked {
            return self
                .data_observed_at
                .map(|t| (now - t).num_seconds().max(0));
        }
        self.last_success_at.map(|t| (now - t).num_seconds().max(0))
    }

    /// Records the actual capture/observation time of the data this observer
    /// just supplied, when that differs from poll time (e.g. a snapshot file
    /// read off disk long after it was written). Once called, freshness
    /// reporting (`last_sync_age_secs`) is driven by this instead of
    /// `last_success_at`.
    ///
    /// `captured_at: None` means the observer knows it should be tracking
    /// data age but could not determine one this poll (missing/invalid
    /// capture timestamp) — never render old data as current, so this forces
    /// the status to `Degraded` (unless already `Down`) with an explanatory
    /// `last_error`, rather than leaving a `Healthy` status paired with an
    /// unknown age.
    pub fn set_data_age(&mut self, captured_at: Option<DateTime<Utc>>) {
        self.data_age_tracked = true;
        self.data_observed_at = captured_at;
        if captured_at.is_none() && self.status != ObserverStatus::Down {
            self.status = ObserverStatus::Degraded;
            self.last_error = Some("snapshot has no captured_at — age unknown".to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_unverified_never_healthy_by_default() {
        let h = ObserverHealth::new("remote_claude");
        assert_eq!(h.status, ObserverStatus::Unverified);
        assert!(h.last_success_at.is_none());
    }

    #[test]
    fn losing_a_capability_is_detectable() {
        let before = CapabilitySet::from_iter(["sessions", "permissions", "worktree"]);
        let after = CapabilitySet::from_iter(["sessions"]);
        let lost = after.lost_since(&before);
        assert_eq!(
            lost,
            vec!["permissions".to_string(), "worktree".to_string()]
        );
    }

    #[test]
    fn partial_capability_loss_is_degraded_not_healthy() {
        // Adversarial finding #3: losing `sessions` while `routines` still
        // parses must NOT read as Healthy just because capabilities is
        // non-empty — it used to have more.
        let mut h = ObserverHealth::new("remote_claude");
        h.record_success(
            Utc::now(),
            CapabilitySet::from_iter(["sessions", "routines"]),
        );
        assert_eq!(h.status, ObserverStatus::Healthy);

        h.record_success(Utc::now(), CapabilitySet::from_iter(["routines"]));
        assert_eq!(
            h.status,
            ObserverStatus::Degraded,
            "lost `sessions` — must not still read Healthy"
        );
        assert!(h.capabilities.has("routines"));
        assert!(!h.capabilities.has("sessions"));
    }

    #[test]
    fn three_consecutive_failures_marks_down_not_just_degraded() {
        let mut h = ObserverHealth::new("remote_claude");
        h.record_failure("timeout");
        assert_eq!(h.status, ObserverStatus::Degraded);
        h.record_failure("timeout");
        assert_eq!(h.status, ObserverStatus::Degraded);
        h.record_failure("timeout");
        assert_eq!(h.status, ObserverStatus::Down);
    }

    #[test]
    fn success_with_zero_capabilities_is_still_down() {
        let mut h = ObserverHealth::new("remote_claude");
        h.record_success(Utc::now(), CapabilitySet::new());
        assert_eq!(h.status, ObserverStatus::Down);
    }
}
