# THE FOUNDRY — Ops Room dashboard (Phase 1-3 scope)

Brey's ambient ops dashboard for his Claude estate. This is a **standalone internal tool**, unrelated to this repo's sports-betting product — it lives here only because this repo/branch is where the harness assigned the work. See the full design plan at `/root/.claude/plans/mission-design-a-live-eventual-russell.md` for the product concept, visual design, and complete phased roadmap.

**Current status: Phase 1-3 (telemetry proof), STOPPED at the truth gate per the authorized scope.** No Pixi/graphical floor exists yet — that's Phase 6+, and is blocked on Brey reviewing this phase's results.

## Layout

- `PHASE1_REPORT.md` — telemetry capability probe + Rust-vs-Bun/Node watcher runtime spike findings (§17 Phase 1).
- `spike/` — the raw Phase 1 spike artifacts: redacted fixture corpus (`spike/fixtures/`), the Rust and Bun/Node prototype binaries used for the runtime measurements. Not part of the shipped watcher core — kept for reference/reproducibility.
- `watcher-core/` — the actual Phase 2/3 deliverable: a Rust crate (`foundry_core`) implementing the normalized event schema, redactor, observer interface + `RemoteClaudeObserver` adapter, state reducer, event log, and a plain-text CLI renderer with `--audit` mode. See `watcher-core/src/observer.rs`'s module doc comment for the current data-access bridge mechanism and its open question (how a real always-on binary would authenticate to the Remote Claude surface — unresolved, flagged for Brey).

## Running it

```
cd watcher-core
cargo test                                          # 23 unit + 8 red-team integration tests
cargo run -- --feed-dir live-feed                    # single-shot render against whatever is in live-feed/
cargo run -- --feed-dir live-feed --audit            # state store vs raw snapshot, side by side
cargo run -- --feed-dir live-feed --watch 30         # poll every 30s (closer to the real always-on mode)
```

`live-feed/` expects `list_sessions.json` and `list_triggers.json` in the shape documented in `src/observer.rs`'s `remote_claude_raw` module — today refreshed manually by a Claude session with the Claude_Code_Remote MCP tools loaded (see the open question above).

## What NOT to build yet

Per the explicit Phase 1-3 authorization: no Pixi/WebGL floor, no procedural assets, no animation, no `.foundry` heartbeat instrumentation in any other repo's scripts, no WebSocket transport (the Rust-in-Tauri finding in `PHASE1_REPORT.md` §2 means one may not even be needed), no project/bay/room resolution (§9, Phase 5). This phase's only job was proving the telemetry pipeline tells the truth.
