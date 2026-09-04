# THE FOUNDRY — Phase 1 Report: Telemetry Capability + Watcher Runtime Spike

Date: 2026-09-04. Scope: design doc §17 Phase 1 only, strictly read-only against every observed Claude project. All writes confined to `ops-room/`.

## 1. Telemetry capability probe

Confirmed §7's core finding: the Remote Claude surface (`list_sessions`, `get_session`, `list_triggers`, `list_environments`, `list_repos`) returns the large majority of the desired telemetry, across both `anthropic_cloud` and `bridge` session kinds, without needing any unsupported/private mechanism.

**Session fields, by kind:**
- **Cloud** (`environment_kind: anthropic_cloud`): status bucket, connection status, configured/current/last-served model, effort level, rate-limit info, `post_turn_summary` (including `needs_action` → BREY_REQUIRED), `context_usage` (present for active sessions), git sources/branches, worktree state when connected.
- **Bridge** (`environment_kind: bridge`): same core fields, **but no `context_usage`** — the context-fill gauge is cloud-only in V1. No `permission_mode` field either. Disconnected bridge machines carry `last_init_error.error_kind` (e.g. `computer_unreachable`), which is the signal that must map to STALE, never IDLE.
- **Archived**: same shape, `status_bucket = COMPLETED`.

**Deltas from the design doc's assumptions:**
- `list_environments` does not enumerate bridge machines — they're only discoverable implicitly via session `tags`/`origin`, not as first-class environment entries.
- Session-bound routines (all of Brey's routines) have no `last_run.outcome` — success must be inferred from the bound session's post-fire activity, exactly as §7 anticipated and §5a's honesty rule requires (rendered `inferred`, never `observed`).

**Cloud per-session event-stream capability: NOT FOUND.** Searched the full deferred-tool list for anything resembling `list_events`/`get_events`/session event streams — none exists today. This confirms §7's V1 assumption: cloud sessions get coarse (snapshot) fidelity only; per-tool animation for cloud sessions stays out of scope until/unless this capability appears. Local sessions (via `~/.claude` JSONL, Phase 2+) remain the only route to fine-grained tool events.

**Fixtures:** captured and redacted per §15 under `ops-room/spike/fixtures/` — `list_sessions.json`, `list_triggers.json`, `list_environments.json`, `list_repos.json`, four `get_session_*.json` samples spanning cloud/bridge/blocked/disconnected, and `event_stream_probe.json` documenting the negative result above.

## 2. Watcher runtime spike: Rust vs Bun/Node — decisive, not close

Built matched, dependency-free prototypes (JSONL tail, git shell-out, idle wait) in both, measured on this machine:

| | Rust (std-only) | Bun (built-in only) | Node 22 |
|---|---|---|---|
| Idle RSS | **2.4 MB** | 43.5 MB | 47.7 MB |
| Binary/bundle size | **508 KB** (release) | 95 MB (`bun build --compile`) | no equivalent first-party compiler (SEA is experimental) |
| Startup (warm) | ~4.6ms (mostly OS fork/exec) | ~20ms | ~35-40ms |
| Local WebSocket server | not attempted (see below) | zero-dep, verified working (`Bun.serve`) | needs the `ws` package — no built-in server |
| Dev speed | slower (hand-rolled JSONL tailer, no JSON parsing tested) | fastest | fast |

**Rust wins on every axis §12a's tie-breaker cares about for an always-on desktop tool**, by 18-20x on idle RSS and ~190x on binary size — not a close call. Bun/Node's advantages (dev speed, a genuine zero-dependency local WebSocket server) matter for iteration speed, not for the thing that runs unattended for hours on Brey's machine.

**A finding that changes the architecture, not just the language choice:** running the watcher *inside* the Tauri process in Rust means it does not need a local WebSocket server at all for its primary consumer (the eventual graphical UI) — Tauri's own `invoke`/`emit` IPC bridge serves that role with no port, no handshake token, and none of §15's "local socket" security surface to build. A standalone WebSocket only becomes relevant later, for an optional browser-based dev/debug client — and building one in dependency-free Rust is genuinely non-trivial (hand-rolling RFC6455 framing, ~400-800 LOC); a real implementation would use a crate like `tungstenite`, which is fine — the "no external crates" constraint was a Phase-1 fairness rule for the spike, not a production constraint.

**Decision: the Phase 2/3 watcher core is built in Rust**, running in-process (no sidecar).

## 3. What this changes for Phase 2/3

- The watcher/state core (schema, redactor, reducer, event log, observer-health) is implemented as a Rust crate at `ops-room/watcher-core/`.
- No WebSocket transport is built in Phase 1-3 — the Phase 3 deliverable (a CLI text renderer) consumes the reducer's state store directly, in-process. A transport layer is deferred to whenever the graphical UI split actually needs one, and per this finding, Tauri's native IPC may replace the WebSocket transport in §8 entirely rather than needing one built.
- A genuine open question, not resolved by this spike, is flagged prominently in the Phase 3 report: **how does a real, always-on Rust binary authenticate to the Remote Claude surface at all**, given every tool probed in Phase 1 is only reachable via MCP tool-calling inside an authenticated Claude session, not a documented public REST endpoint. This is the single biggest unresolved architecture question before V1 can ship as a standalone desktop app.
