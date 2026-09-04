# WEEKEND PROGRESS — THE FOUNDRY (append-only)

## 2026-09-04T21:25Z — weekend mode start

**Completed**
- R-01: `foundry-extracted` (subtree split, 5 commits, authorship/timestamps preserved) pushed to `breydenerrett-cmd/TheFoundry` as `main` @ `db570b9`. Fresh clone builds; `cargo test` = 32 unit + 3 heartbeat + 4 local-observer + 16 red-team = **55/55 pass**.
- R-02: aisportsanalysis PR #1 closed (not merged) with extraction note. Sports repo untouched: branch `claude/ops-room-dashboard-design-jnqhxl` still at `91b0664`, clean working tree.
- W-01: control files created.

**Blocked (Brey, recorded once)**
- R-03/R-04 rename → `the-foundry` + PRIVATE: the session's GitHub proxy returns `403 "Repository settings writes are not permitted through this proxy"`; the GitHub MCP toolset has no rename/visibility tool; this sandbox has no browser control. Two clicks in repo Settings on Brey's side.
- G-01 `gh` on the Windows machine: this session cannot reach Brey's machine. `winget install GitHub.cli && gh auth login` locally, then a local Claude Code session can administer via `gh`.

**GitHub ops autonomous from this session (G-02)**
- Can: push branches, open/close/update PRs, PR comments/reviews, create branches, read/write files via API, trigger/list Actions, read releases, subscribe to PR events.
- Cannot: create repos (403), rename, change visibility/settings, manage collaborators, create releases (untested — settings-class write, assume no), anything requiring a browser.

**Model usage decisions**
- Fable for orchestration only; implementation tasks will be dispatched to Sonnet workers; Haiku for inspection; Opus reserved for O-01.

**Next**
- W-02 create hourly Routine; then T-01 truth-gate re-verification; then T-02 CI.

**Security/resource notes**
- Repo is currently PUBLIC. Verified no secrets in tree (redactor tests + earlier scans). No tokens were printed anywhere in this session.

## 2026-09-04T21:50Z — T-01 truth gate + T-02 CI

**Completed**
- T-01: live text-mode run (fixture remote feed + real git + heartbeat file) exercised every observer, bay grouping, output velocity, redaction (paths in heartbeat `error` and session labels → `[REDACTED-SECRET]`), observer health, 3 BREY_REQUIRED, HUNG, STALE/UNKNOWN. **Found a truth bug**: `REMOTE ESTATE: live (last sync 0s)` rendered for a snapshot captured at 13:40Z because sync age measured *file read* time. Fixed: snapshot capture time now comes from `<feed-dir>/captured_at` (or a top-level `captured_at` field); missing/future-skewed → observer Degraded + "REMOTE ESTATE — DEGRADED". Proof after fix: `REMOTE ESTATE — DEGRADED (last sync 7h51m ago …)`. +3 red-team tests → **58/58**. Compiler warnings cleared; `cargo clippy -D warnings` and `cargo fmt --check` clean (workspace-wide fmt applied — formatting-only diff).
- T-02: `.github/workflows/ci.yml` — fmt, clippy, test, plus a smoke step asserting `--no-remote` never prints "REMOTE ESTATE: live".
- W-02 → BLOCKED_EXTERNAL: `create_trigger` and `send_later` both denied by the permission classifier; loop persists only within this turn.

**Model usage**: one Sonnet worker (~106k tokens) for the fix; Fable orchestration.
**Next**: P-01 project resolution v2 (UNRESOLVED bay, explicit map), then S-01 persistence.

## 2026-09-04T22:05Z — P-01 project resolution v2

- `BayMap` loaded from `foundry.bays.toml` (`--bay-map PATH`): explicit `[repos]` identity (owner/name from git URL or path basename) → ordered `[rules]` substrings → `[tags]` → **UNRESOLVED**. The old defaults (`None` → PERSONAL/MISC, unmatched → EXPERIMENTS) were guesses and are gone; UNRESOLVED renders as its own last group `▸ UNRESOLVED (no repo/tag match — not guessed)`. Missing map file → everything UNRESOLVED + stderr note, never an error.
- Tests 58 → **64**; clippy/fmt clean. New dep: `toml`.
- Known gap unchanged: the remote fixture carries no repo URL, so remote sessions are UNRESOLVED until the refresh snapshot includes `sources[].git_repository.url`.
- Sonnet worker ~113k tokens. Next: S-01 persistence; H-01 running in parallel (separate repo worktree).

## 2026-09-04T22:15Z — H-01 sports heartbeats (code complete, merge needs Brey)

- Added `scripts/foundry_beat.sh` / `.py` + one-line beats in forward_capture, daily_loop, monitor_remote, test_parallel on a new branch off the live sports branch; draft PR https://github.com/breydenerrett-cmd/aisportsanalysis/pull/2. Helper is fail-safe (subshell, `|| true`, `set -u` safe), writes only fixed reasons / HTTP classes / pass counts. Verified with bash -n, py_compile, JSON validation, and a Foundry read of the emitted file.
- Not merged autonomously: the base branch has routines and workers executing against it; merging is a safe-boundary decision for Brey. Real-event proof therefore pending → BLOCKED_HUMAN.
- Sonnet worker ~98k tokens. S-01 persistence worker running.

## 2026-09-04T22:40Z — S-01 persistent state

- `persist.rs` + seq-numbered `PersistedEvent` envelope in the JSONL log; atomic `snapshot.json`; `--no-restore`. Restart truth rule enforced: restored sessions/routines/checks are tagged `(restored)` and forced STALE/UNKNOWN, observer health not restored, `pipeline_verified` false until canary + a real observer fire post-restart; per-record staleness persists until that entity is re-observed. TTL expiry (`DEFAULT_SESSION_TTL_SECS=900`). Corrupted snapshot → warning, not crash.
- Bug found by the restart demo: `derive_stalls` re-derived HUNG over restored records — fixed (skips restored).
- Tests 64 → **74**; clippy/fmt clean. Sonnet worker ~189k tokens (largest so far).
- Next: M-01 multi-machine agent; V-01 visual foundation running in `app/`.

## 2026-09-04T23:05Z — V-01 static visual foundation

- `app/` (Vite + React 18 + PixiJS 8, TypeScript): isometric SUBSTRATE floor with marquee, 6 bays + hatched UNRESOLVED, stations for all 12 states (+ fading/ended), observed = solid / inferred = dashed / unknown = hatched, per-bay output shelf where uninstrumented slots are hatched (UNKNOWN ≠ 0), FAILED/HUNG red and BREY_REQUIRED amber beacons, and a scanline "blind" overlay whenever pipeline is unverified or remote is degraded. `#scene-mirror` DOM exposes id/state/fidelity per station for machine assertion.
- Playwright 4/4 (state mapping, overlay, marquee count, screenshot at `app/screenshots/floor-fixture.png`). CI gained a `web` job (build + Playwright). Tauri is scaffolded only — no webkit2gtk in this sandbox.
- Honest assessment: a correct foundation, not yet a polished visual — stations are small primitives; density/motion/typography come in V-02..V-05.
- Sonnet worker ~110k tokens. M-01 worker running.

## 2026-09-04T23:30Z — M-01 multi-machine foundation

- `foundry-agent` (second binary) runs the same zero-token local observers per machine and publishes HMAC-SHA256-signed bundles (`sign.rs`, key from `FOUNDRY_AGENT_KEY`/`--key-file` only, never logged) over a replaceable transport (`transport.rs`; file transport built, HTTP push documented as next). Main side `AgentIngestObserver` rejects bad signature / unknown key id / replayed seq / >5 min skew — each as visible per-agent degradation, never a silent drop. New `MACHINES` render section; silent agents past TTL leave sessions STALE/UNKNOWN, never stuck WORKING. Redaction applied again before signing (tested).
- Tests 74 → **88**; clippy/fmt clean; both binaries build.
- CI: run #5 failed (`vite preview` bound to ::1 on Node 24 runners vs 127.0.0.1 baseURL) — fixed in 992687f by `--host 127.0.0.1`.
- Sonnet worker ~181k tokens. V-02 worker running in `app/`.

## 2026-09-04T23:50Z — V-02 three-second glance

- Floor now reads at a glance: 20px marquee in the required order (`1 BREY REQUIRED — <station>` brightest amber), ambient luminance tracks WORKING/THINKING count, chunky isometric chassis with light pools and signage, spark/drift/plume particles under a global budget, ghost-outline motion for inferred fidelity and none for unknown, BREY/FAILED/HUNG beacons, HUNG elapsed counter, STALE "LAST SYNC" tag, Opus violet chamber + `N OPUS`. Idle and blind fixtures prove: no overlay + dim ambient when genuinely idle and verified; overlay + UNVERIFIED when blind.
- Playwright 4 → **8**. Screenshots: `floor-fixture.png`, `floor-idle.png`, `floor-blind.png`.
- Honest critique (worker's and mine): still schematic — flat shading, no bloom/CRT texture, and the whole floor sits under-scaled in the upper-left ~50% of the viewport. V-03 must fit-to-viewport and add a surface-detail pass.
- CI #6 green after the preview-host fix. Sonnet worker ~118k tokens. O-01 Opus review running.

## 2026-09-05T00:25Z — V-03 station states + fit-to-viewport

- `src/states.ts` is now the single state→{color, light, motion, glyph, beacon} table; `restored` forces STALE treatment regardless of state. Fit-to-viewport layout (content-only bounds, ~92% fill, tested at 1280×720 and 1920×1080 via `data-scene-bounds`), 3×3 bay grid, gradient/rim-light/scanline/furniture pass. `floor-states.json` fixture with all 13 states + inferred + restored. Playwright 8 → **10**.
- Carry-over into V-04: label collisions at small stations; glyph drawing still uses the old switch, not `spec.glyph`.
- Sonnet worker ~156k tokens. O-01 fixer running in watcher-core.

## 2026-09-05T00:45Z — O-01 adversarial review + fixes

- Opus (single authorized use, ~125k tokens) confirmed 7 lies with failing tests (`tests/opus_review.rs`): stale-log replay after restart rendered WORKING; future-dated heartbeat fabricated `LAST OUTPUT 0s`; seq-0 agent bundle replayable forever (dead machine → REACHABLE); Degraded observer with empty capabilities still "pipeline verified"; any source's heartbeat certified the canary and un-restored the floor; stale routine advertised as NEXT; agents observer HEALTHY with an unreadable transport dir.
- Sonnet fixer (~224k tokens) fixed all 7 + security hardening: key_id↔agent_id binding (`--agent-keys agent_id=key_id=path`), unverified rejections bucketed as "unverified" not the claimed id, `last_error`/rejection reasons redacted before storage, per-agent seq watermarks persisted in the snapshot, `last_heard_at` from `sent_at`. One existing test was corrected because it asserted the vulnerable behavior.
- Judgment call recorded: the `last_seq()` off-by-one Opus flagged was NOT changed — fixer traced main.rs and found snapshot-save always follows log-append, and the persistence test locks the exclusive bound intentionally. Revisit if the poll loop ever changes.
- Tests 88 → **100**; clippy/fmt clean; both binaries build.
