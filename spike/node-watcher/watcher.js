#!/usr/bin/env node
// Minimal watcher runtime spike — plain JS, Node/Bun built-ins ONLY (no npm deps).
// Stands in for "the watcher runtime, implemented as a packaged Bun/Node sidecar"
// per design doc §12a (option B), to be measured head-to-head against the Rust
// observer/backend option (A).
//
// Runs on either `node` (v22+) or `bun` unmodified — no runtime-specific APIs.

'use strict';

const fs = require('fs');
const path = require('path');
const { execFileSync } = require('child_process');

const REPO_ROOT = process.env.WATCHER_REPO_ROOT || '/home/user/aisportsanalysis';
const JSONL_PATH =
  process.env.WATCHER_JSONL_PATH ||
  path.join(__dirname, '..', 'fixtures', 'sample_events.jsonl');
const TOTAL_LIFETIME_MS = Number(process.env.WATCHER_LIFETIME_MS || 15000);
const POLL_INTERVAL_MS = 200;

// --- 0. Print READY as soon as the process is initialized (startup-time marker) ---
console.log('READY');

// --- 1. Tail a JSONL file for appended lines ---
// Uses a portable poll-based tail (stat + read-from-offset) so it behaves
// identically on Node and Bun and on filesystems where fs.watch is flaky
// (network mounts, some containers). fs.watch is layered on top where
// available, purely to react faster; the poll loop is the source of truth.
function tailJsonl(filePath, onLine) {
  let offset = 0;

  function readNewBytes() {
    let stat;
    try {
      stat = fs.statSync(filePath);
    } catch (err) {
      return; // file not there yet
    }
    if (stat.size < offset) {
      // File was truncated/rotated — start over.
      offset = 0;
    }
    if (stat.size > offset) {
      const fd = fs.openSync(filePath, 'r');
      const len = stat.size - offset;
      const buf = Buffer.alloc(len);
      fs.readSync(fd, buf, 0, len, offset);
      fs.closeSync(fd);
      offset = stat.size;
      const chunk = buf.toString('utf8');
      for (const line of chunk.split('\n')) {
        const trimmed = line.trim();
        if (trimmed.length > 0) onLine(trimmed);
      }
    }
  }

  // Prime: read whatever is already in the file (initial tail catch-up).
  readNewBytes();

  // Best-effort fs.watch for low-latency reaction to appends.
  let watcher = null;
  try {
    watcher = fs.watch(filePath, { persistent: false }, () => readNewBytes());
  } catch (err) {
    // fs.watch can throw (e.g. file missing, platform limits) — poll covers us.
  }

  // Polling safety net — guarantees correctness even if fs.watch misses events
  // (coalesced writes, editors that replace-via-rename, etc).
  const pollTimer = setInterval(readNewBytes, POLL_INTERVAL_MS);
  pollTimer.unref && pollTimer.unref();

  return () => {
    clearInterval(pollTimer);
    if (watcher) watcher.close();
  };
}

console.log(`[jsonl] tailing ${JSONL_PATH}`);
const stopTail = tailJsonl(JSONL_PATH, (line) => {
  console.log(`[jsonl:line] ${line}`);
});

// Append a synthetic line shortly after startup so the tail path is
// exercised end-to-end in this same process, not just against pre-existing
// fixture content.
setTimeout(() => {
  try {
    const marker = JSON.stringify({
      ts: new Date().toISOString(),
      type: 'watcher_self_test',
      note: 'appended by node-watcher spike to exercise the tail path',
    });
    fs.appendFileSync(JSONL_PATH, marker + '\n');
  } catch (err) {
    console.log(`[jsonl] append self-test failed: ${err.message}`);
  }
}, 500);

// --- 2. Shell out to git, read-only ---
function gitInfo(repoRoot) {
  const short = execFileSync('git', ['status', '--short'], {
    cwd: repoRoot,
    encoding: 'utf8',
  });
  const branch = execFileSync('git', ['rev-parse', '--abbrev-ref', 'HEAD'], {
    cwd: repoRoot,
    encoding: 'utf8',
  }).trim();
  const dirty = short.trim().length > 0;
  return { branch, dirty, shortLineCount: short.trim().length ? short.trim().split('\n').length : 0 };
}

try {
  const { branch, dirty, shortLineCount } = gitInfo(REPO_ROOT);
  console.log(`[git] branch=${branch} dirty=${dirty} changed_paths=${shortLineCount}`);
} catch (err) {
  console.log(`[git] failed: ${err.message}`);
}

// --- 3. Idle so this process can be measured at rest, then exit on a timer ---
console.log(`[idle] entering idle wait for ${TOTAL_LIFETIME_MS}ms total lifetime`);
setTimeout(() => {
  stopTail();
  console.log('[exit] lifetime elapsed, shutting down');
  process.exit(0);
}, TOTAL_LIFETIME_MS);
