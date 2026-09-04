//! Phase 4B — integration tests for the `.foundry/events.jsonl` heartbeat
//! contract observer.

use chrono::Utc;
use foundry_core::heartbeat::HeartbeatObserver;
use foundry_core::observer::Observer;
use std::fs;

#[test]
fn reads_real_heartbeat_lines_and_redacts_errors() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join(".foundry")).unwrap();
    let now = Utc::now();
    let lines = format!(
        "{{\"component\":\"forward_capture\",\"event\":\"end\",\"status\":\"ok\",\"ts\":\"{}\",\"artifact\":\"3 files changed\"}}\n\
         {{\"component\":\"daily_loop\",\"event\":\"end\",\"status\":\"escalate\",\"ts\":\"{}\",\"error\":\"credit floor hit at /home/user/secretproj\"}}\n",
        now.to_rfc3339(), now.to_rfc3339()
    );
    fs::write(dir.path().join(".foundry/events.jsonl"), lines).unwrap();

    let mut obs = HeartbeatObserver::new(dir.path(), "SPORTS LAB");
    let events = obs.poll(now);
    assert_eq!(events.len(), 2);
    let escalate = events.iter().find(|e| e.label.as_deref().unwrap().contains("daily_loop")).unwrap();
    assert!(escalate.label.as_deref().unwrap().contains("ESCALATE"));
    assert!(!escalate.label.as_deref().unwrap().contains("/home/user/secretproj"), "path in error field must be redacted");
    assert_eq!(obs.health().status, foundry_core::health::ObserverStatus::Healthy);
}

#[test]
fn one_malformed_line_does_not_lose_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join(".foundry")).unwrap();
    let now = Utc::now();
    let lines = format!(
        "not json at all\n\
         {{\"component\":\"test_runner\",\"event\":\"end\",\"status\":\"ok\",\"ts\":\"{}\"}}\n",
        now.to_rfc3339()
    );
    fs::write(dir.path().join(".foundry/events.jsonl"), lines).unwrap();

    let mut obs = HeartbeatObserver::new(dir.path(), "SPORTS LAB");
    let events = obs.poll(now);
    assert_eq!(events.len(), 1);
    assert_eq!(obs.health().status, foundry_core::health::ObserverStatus::Healthy);
}

#[test]
fn missing_file_degrades_honestly_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let mut obs = HeartbeatObserver::new(dir.path(), "SPORTS LAB");
    let events = obs.poll(Utc::now());
    assert!(events.is_empty());
    assert_ne!(obs.health().status, foundry_core::health::ObserverStatus::Healthy);
}
