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
