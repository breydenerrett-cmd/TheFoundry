# WEEKEND AUTONOMOUS PLAN — THE FOUNDRY

Started: 2026-09-04 (Brey away for the weekend; no routine approvals available).

## Objective
Advance THE FOUNDRY from the Phase 4 truth-layer state toward a polished, standalone, always-on ops dashboard — truthfulness before polish, never confident-green without observability.

## Operating loop (every invocation)
1. Read `WEEKEND_QUEUE.md`.
2. Inspect repo/git/worktree state (`git status`, `git log -3`, `cargo test`).
3. Check for a live worker: `WEEKEND_QUEUE.md` rows in `RUNNING` with a `started` timestamp < 90 min old are considered active; older `RUNNING` rows are reclaimed → `READY` (evidence noted).
4. Pick the highest-priority `READY` task whose dependencies are `DONE`.
5. Mark `RUNNING` (with timestamp), execute, verify, commit, push, mark `DONE` with SHA.
6. Append to `WEEKEND_PROGRESS.md`.
7. Continue to the next `READY` task if budget allows; otherwise exit cleanly.

## Model routing
- **Fable** — orchestration, task selection, synthesis, queue updates.
- **Haiku** — reads, searches, doc checks, simple validation.
- **Sonnet** — all implementation (Rust, React, Tauri, Pixi, tests).
- **Opus** — only for a narrow adversarial/security review at a milestone where a hidden defect would be costly (the visual truth-mapping gate).
- Deterministic tools (cargo test/clippy/fmt, scripts) preferred over model reasoning.

## Environment facts (established, don't re-probe)
- This session runs in a cloud sandbox. **No Chrome/browser control, no access to Brey's Windows machine, no `gh` CLI.** GitHub reaches through an agent proxy that permits git push and MCP PR/issue/contents ops but rejects repository-settings writes (`"Repository settings writes are not permitted through this proxy"`) and org-level repo creation (403).
- Standalone repo: https://github.com/breydenerrett-cmd/TheFoundry (default branch `main`). Rename → `the-foundry` and PRIVATE need Brey (Settings → General → rename; Danger Zone → Change visibility). Until then the repo is PUBLIC — no secrets are in it (redaction verified), but treat it as public.
- Sports scripts (`forward_capture.sh`, `daily_loop.sh`, `monitor_remote.sh`, `test_parallel.py`) live on `claude/sports-betting-analysis-review-g1o0co` in `aisportsanalysis`, not on any branch this session owns.

## Persistence of the loop
Preferred: a Claude Routine `Foundry Weekend Build Loop` firing hourly into this session (see `WEEKEND_QUEUE.md` row W-02 for the outcome). Fallback if the Routine cannot be created: `send_later` self check-ins chained one hour apart from each invocation.

## Hard rules
- No secrets/tokens in prompts, logs, commits, or docs.
- No history rewrites, no repo deletion/transfer, no visibility → public, no account-wide security changes.
- Max 3 automated retries on transient failure; no unbounded loops.
- Absence of data renders as UNKNOWN/DEGRADED, never healthy.
- Stop all work only if every remaining task needs Brey, the repo state is unsafe, credentials are at risk, or usage limits block execution.
