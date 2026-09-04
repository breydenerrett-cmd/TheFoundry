// foundry_watcher_rs
//
// Minimal std-library-only Rust prototype standing in for "the watcher
// runtime, implemented in Rust inside the Tauri process" (design doc §12a).
// No external crates: this must build with zero network/registry access so
// the build-time and binary-size comparison against the Node/Bun sidecar
// spike is apples-to-apples on raw runtime cost, not on dependency weight.
//
// What it does, in order:
//   1. Prints "READY" to stdout the instant startup work is done, so an
//      external harness can time process-spawn -> READY as "startup time".
//   2. Tails a JSONL file by polling its size every ~200ms with std::fs
//      (no notify crate, no inotify) and prints any newly-appended lines.
//   3. Shells out to `git status --short` and
//      `git rev-parse --abbrev-ref HEAD` against a target repo (read-only)
//      via std::process::Command, and prints the branch + dirty flag.
//   4. Idles (thread::sleep) so it can be measured at rest, continuing to
//      poll the JSONL file the whole time, until a fixed total lifetime
//      (default 15s) elapses, then exits cleanly.

use std::env;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

/// Total process lifetime before self-exit, in seconds.
const TOTAL_LIFETIME_SECS: u64 = 15;
/// JSONL poll interval.
const POLL_INTERVAL_MS: u64 = 200;

fn main() {
    let run_start = Instant::now();

    // --- Args (all optional; sensible defaults for the spike) ---------
    // arg1: path to the JSONL file to tail
    // arg2: path to the git repo to interrogate
    let mut args = env::args().skip(1);
    let jsonl_path: PathBuf = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/home/user/aisportsanalysis/ops-room/spike/fixtures/sample_events.jsonl"));
    let repo_path: PathBuf = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/home/user/aisportsanalysis"));

    // --- Step 0: signal readiness as early as physically possible ------
    // Nothing above this point does any I/O or blocking work, so this is
    // as close to "process init done" as the prototype can measure.
    println!("READY");
    // Explicit flush: stdout is line-buffered when attached to a TTY but
    // fully buffered when piped (which is exactly how a startup-time
    // harness will be capturing it), so an unflushed buffer would make
    // "READY" invisible to a reader until much later.
    io::stdout().flush().ok();

    let ready_at = run_start.elapsed();
    eprintln!("[watcher] init -> READY in {:?}", ready_at);

    // --- Step 2: one-shot read-only git interrogation -------------------
    // (Done before starting the poll loop so the loop's timing is clean;
    // git status/rev-parse on a small-to-medium repo is single-digit ms.)
    run_git_probe(&repo_path);

    // --- Step 1 + 4: JSONL tail-by-polling, running for the process's
    // idle lifetime so it can be observed both "doing work" (new lines
    // arriving) and "at rest" (nothing arriving, just sleeping) by the
    // same loop -- which is the realistic shape of the real watcher.
    let mut tailer = JsonlTailer::new(&jsonl_path);
    tailer.drain_existing(); // print whatever is already in the file

    eprintln!(
        "[watcher] tailing {} (poll every {}ms), idling until {}s total lifetime",
        jsonl_path.display(),
        POLL_INTERVAL_MS,
        TOTAL_LIFETIME_SECS
    );

    let lifetime = Duration::from_secs(TOTAL_LIFETIME_SECS);
    let poll_every = Duration::from_millis(POLL_INTERVAL_MS);

    loop {
        let elapsed = run_start.elapsed();
        if elapsed >= lifetime {
            break;
        }
        tailer.poll_and_print();
        // Sleep either a full poll interval, or whatever's left of the
        // total lifetime, whichever is shorter, so we exit on schedule.
        let remaining = lifetime - elapsed;
        std::thread::sleep(poll_every.min(remaining));
    }

    eprintln!(
        "[watcher] lifetime ({}s) elapsed, exiting cleanly",
        TOTAL_LIFETIME_SECS
    );
}

/// Shell out to git (read-only) and print branch + dirty flag.
fn run_git_probe(repo_path: &PathBuf) {
    let status_out = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("status")
        .arg("--short")
        .output();

    let branch_out = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("rev-parse")
        .arg("--abbrev-ref")
        .arg("HEAD")
        .output();

    match (status_out, branch_out) {
        (Ok(status_out), Ok(branch_out)) => {
            let branch = String::from_utf8_lossy(&branch_out.stdout)
                .trim()
                .to_string();
            let short_status = String::from_utf8_lossy(&status_out.stdout);
            let dirty = !short_status.trim().is_empty();
            let changed_lines = short_status.lines().count();

            println!(
                "GIT branch={} dirty={} changed_files={}",
                if branch.is_empty() { "?".to_string() } else { branch },
                dirty,
                changed_lines
            );
            if dirty {
                for line in short_status.lines().take(10) {
                    println!("GIT   {}", line);
                }
            }
        }
        (status_res, branch_res) => {
            eprintln!(
                "[watcher] git probe failed: status_ok={} branch_ok={}",
                status_res.is_ok(),
                branch_res.is_ok()
            );
        }
    }
    io::stdout().flush().ok();
}

/// Polls a file's size via std::fs and prints newly-appended, newline-
/// terminated lines. Buffers a trailing partial line across polls so a
/// write that lands mid-line doesn't get split and printed twice.
struct JsonlTailer {
    path: PathBuf,
    offset: u64,
    partial: Vec<u8>,
}

impl JsonlTailer {
    fn new(path: &PathBuf) -> Self {
        JsonlTailer {
            path: path.clone(),
            offset: 0,
            partial: Vec::new(),
        }
    }

    /// Print whatever is already in the file at startup, then leave the
    /// offset at EOF so the poll loop only reports genuinely new lines.
    fn drain_existing(&mut self) {
        self.poll_and_print();
    }

    /// Check current size via std::fs::metadata; if it grew, read the new
    /// bytes and print any complete lines found.
    fn poll_and_print(&mut self) {
        let meta = match fs::metadata(&self.path) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("[watcher] jsonl stat error ({}): {}", self.path.display(), e);
                return;
            }
        };
        let len = meta.len();

        if len < self.offset {
            // File was truncated/rotated; restart from the top.
            eprintln!("[watcher] jsonl shrank ({} -> {}), restarting tail", self.offset, len);
            self.offset = 0;
            self.partial.clear();
        }

        if len == self.offset {
            return; // nothing new
        }

        let mut file = match File::open(&self.path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("[watcher] jsonl open error: {}", e);
                return;
            }
        };
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return;
        }

        let mut buf = Vec::new();
        if file.read_to_end(&mut buf).is_err() {
            return;
        }
        self.offset = len;

        self.partial.extend_from_slice(&buf);

        // Split on '\n', keep any trailing partial line for next time.
        let mut lines: Vec<Vec<u8>> = Vec::new();
        {
            let mut start = 0usize;
            for (i, b) in self.partial.iter().enumerate() {
                if *b == b'\n' {
                    lines.push(self.partial[start..i].to_vec());
                    start = i + 1;
                }
            }
            self.partial = self.partial[start..].to_vec();
        }

        let mut printed_any = false;
        for line in lines {
            let text = String::from_utf8_lossy(&line);
            let text = text.trim_end_matches('\r');
            if !text.is_empty() {
                println!("JSONL {}", text);
                printed_any = true;
            }
        }
        if printed_any {
            io::stdout().flush().ok();
        }
    }
}
