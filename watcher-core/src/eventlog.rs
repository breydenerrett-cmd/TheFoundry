//! Ring buffer + rotating on-disk JSONL log (§8, §13). Every event that
//! reaches here has already passed through redaction — nothing here should
//! ever need to scrub anything again.

use crate::schema::Event;
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct EventLog {
    ring: VecDeque<Event>,
    ring_capacity: usize,
    dir: PathBuf,
    current_date: String,
    current_file: Option<File>,
    retention_files: usize,
}

impl EventLog {
    pub fn new(dir: impl AsRef<Path>, ring_capacity: usize, retention_files: usize) -> std::io::Result<Self> {
        fs::create_dir_all(&dir)?;
        Ok(Self {
            ring: VecDeque::with_capacity(ring_capacity.min(1 << 20)),
            ring_capacity,
            dir: dir.as_ref().to_path_buf(),
            current_date: String::new(),
            current_file: None,
            retention_files,
        })
    }

    pub fn append(&mut self, events: &[Event]) -> std::io::Result<()> {
        for ev in events {
            self.ring.push_back(ev.clone());
            while self.ring.len() > self.ring_capacity {
                self.ring.pop_front();
            }
            self.write_to_disk(ev)?;
        }
        Ok(())
    }

    fn write_to_disk(&mut self, ev: &Event) -> std::io::Result<()> {
        let date = ev.ts.format("%Y-%m-%d").to_string();
        if date != self.current_date || self.current_file.is_none() {
            self.rotate(&date)?;
        }
        if let Some(f) = &mut self.current_file {
            let line = serde_json::to_string(ev).unwrap_or_default();
            writeln!(f, "{line}")?;
        }
        Ok(())
    }

    fn rotate(&mut self, date: &str) -> std::io::Result<()> {
        self.current_date = date.to_string();
        let path = self.dir.join(format!("events-{date}.jsonl"));
        let f = OpenOptions::new().create(true).append(true).open(path)?;
        self.current_file = Some(f);
        self.prune_old_files()?;
        Ok(())
    }

    fn prune_old_files(&self) -> std::io::Result<()> {
        let mut files: Vec<PathBuf> = fs::read_dir(&self.dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|e| e == "jsonl").unwrap_or(false))
            .collect();
        files.sort();
        if files.len() > self.retention_files {
            let excess = files.len() - self.retention_files;
            for f in &files[..excess] {
                let _ = fs::remove_file(f);
            }
        }
        Ok(())
    }

    pub fn ring_snapshot(&self) -> Vec<Event> {
        self.ring.iter().cloned().collect()
    }

    pub fn ring_len(&self) -> usize {
        self.ring.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{EntityRef, EntityType, EventKind, Fidelity, Metrics};
    use chrono::{TimeZone, Utc};

    fn ev(ts: chrono::DateTime<Utc>) -> Event {
        Event {
            ts,
            source: "test".into(),
            kind: EventKind::Heartbeat,
            entity: EntityRef::new(EntityType::Project, "p1"),
            project_id: None,
            session_id: None,
            model: None,
            model_current: None,
            model_last_served: None,
            effort: None,
            state: None,
            label: None,
            detail: None,
            fidelity: Fidelity::Observed,
            metrics: Metrics::default(),
            ttl_secs: None,
            next_run_at: None,
            enabled: None,
        }
    }

    #[test]
    fn ring_buffer_caps_at_capacity() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = EventLog::new(dir.path(), 3, 30).unwrap();
        let base = Utc.with_ymd_and_hms(2026, 9, 4, 0, 0, 0).unwrap();
        for i in 0..10 {
            log.append(&[ev(base + chrono::Duration::seconds(i))]).unwrap();
        }
        assert_eq!(log.ring_len(), 3);
    }

    #[test]
    fn rotation_creates_a_dated_file_per_day() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = EventLog::new(dir.path(), 100, 30).unwrap();
        let day1 = Utc.with_ymd_and_hms(2026, 9, 4, 12, 0, 0).unwrap();
        let day2 = Utc.with_ymd_and_hms(2026, 9, 5, 0, 0, 1).unwrap();
        log.append(&[ev(day1)]).unwrap();
        log.append(&[ev(day2)]).unwrap();
        let files: Vec<_> = fs::read_dir(dir.path()).unwrap().filter_map(|e| e.ok()).collect();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn retention_prunes_oldest_files_first() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = EventLog::new(dir.path(), 100, 2).unwrap();
        for day in 1..=5u32 {
            let ts = Utc.with_ymd_and_hms(2026, 9, day, 0, 0, 0).unwrap();
            log.append(&[ev(ts)]).unwrap();
        }
        let files: Vec<_> = fs::read_dir(dir.path()).unwrap().filter_map(|e| e.ok()).collect();
        assert_eq!(files.len(), 2, "only the retention_files-most-recent day files should remain");
    }
}
