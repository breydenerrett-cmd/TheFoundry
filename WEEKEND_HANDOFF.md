# WEEKEND HANDOFF — THE FOUNDRY

Repo: https://github.com/breydenerrett-cmd/TheFoundry (default branch `main`, head `611e2e9`, CI green)

## Starting state (Fri 2026-09-04 ~21:20Z)
- Foundry lived only in `aisportsanalysis` PR #1 (`ops-room/`), unmerged. Rust truth layer at Phase 4: 55 tests, text renderer, fixture-fed remote feed, local/git/heartbeat observers, no UI, no persistence, no multi-machine, no live bridge.
- `breydenerrett-cmd/TheFoundry` existed, PUBLIC, empty.

## Final state (Sat 2026-09-05 ~02:45Z)
- **Standalone repo** with the full extracted history (5 original commits, authorship intact) + 17 weekend commits.
- **Rust watcher (`watcher-core`)**: **108 tests**, clippy/fmt clean, two binaries (`foundry`, `foundry-agent`). CI: fmt + clippy + test + an honesty smoke step.
- **Pixi dashboard (`app/`)**: Vite + React + PixiJS, **27 Playwright tests**, CI web job. Runs LIVE from the watcher (`scripts/dev.sh`), five display modes, all 12 states + fading, honest DOWN/STALE/FIXTURE feed states.
- **Sports heartbeats**: draft PR [aisportsanalysis#2](https://github.com/breydenerrett-cmd/aisportsanalysis/pull/2) — ready, not merged (see blockers). PR #1 closed with an extraction note; sports repo untouched.

## Everything completed (queue IDs)
| ID | What | Commit |
|---|---|---|
| R-01/R-02 | History-preserving push to standalone repo; PR #1 closed; sports repo clean | db570b9 |
| W-01 | Weekend control files | 73b090d |
| T-01 | Truth-gate re-verification; **found+fixed**: snapshot age measured file-read time → "live" for a 7h-old snapshot; now `captured_at`-driven, missing → DEGRADED | 57c81b4 |
| T-02 | GitHub Actions CI (Rust + web) | 57c81b4, 992687f, 0894be3 |
| P-01 | Bay map `foundry.bays.toml`: repos → rules → tags → **UNRESOLVED** (no more guessing) | c9ba525 |
| H-01 | Heartbeat helper + beats in 4 sports scripts (PR #2) | sports 2d20f7c |
| S-01 | Seq-numbered log, atomic snapshot, honest restart (restored → STALE until re-observed), TTLs | 0eb2b0e |
| M-01 | `foundry-agent`, HMAC-signed bundles, file transport, MACHINES section, agent TTL | 4564091 |
| O-01 | Opus adversarial review: **7 confirmed lies fixed** with kept repro tests + 4 security hardenings | fac89be |
| V-01…V-05 | Visual floor: foundation → glance layer → states/fit → modes → performance | 6e41372…a5fbc99 |
| L-01 | Live bridge: `--serve`/`--state-json` on the watcher; app polls it; no fixture fallback | 6557409, c33775d |

## How to see it
```bash
git clone https://github.com/breydenerrett-cmd/TheFoundry && cd TheFoundry
./scripts/dev.sh            # builds watcher, serves 127.0.0.1:8790, opens Vite dev server
# text mode: cd watcher-core && cargo run -- --no-remote --git-dir .. --bay-map ../foundry.bays.toml
# with a remote snapshot: add --feed-dir live-feed  (renders DEGRADED, age from live-feed/captured_at)
# hotkeys in the app: 1 Command Center · 2 Project Focus (←/→, click a bay) · 3 Ambient · 4 Incident · 5 Deep Debug · Esc
```
Screenshots (committed): `app/screenshots/live-floor.png` (real watcher), `live-down.png`, `floor-fixture.png`, `floor-states.png`, `floor-idle.png`, `floor-blind.png`, `mode-*.png`.

## Test totals
Rust 55 → **108** · Playwright 0 → **27** · CI green on the final head (run on 611e2e9 passed: Rust 108 tests, Playwright 27); two post-handoff test fixes were CI-only environment assumptions (empty estate on runners; ambient first-frame timing).

## Architecture changes
- `persist.rs` (snapshot + seq), `export.rs` + `httpd.rs` (FloorState JSON, loopback HTTP), `agents.rs`/`sign.rs`/`transport.rs` (multi-machine), `bay.rs` v2 (BayMap), marquee derivations moved into shared reducer methods so text and JSON cannot disagree.
- App: `states.ts` single state table, `modes.ts`, `perf.ts`, `feed.ts` with liveness.
- Tauri: scaffold only (`app/src-tauri`), not built — no webkit2gtk in the sandbox.

## Known bugs / honest gaps
- Remote sessions all land in UNRESOLVED: the snapshot fixture carries no repo URL. A refreshed snapshot with `sources[].git_repository.url` fixes it via the existing map.
- Output shelf is exported as UNKNOWN everywhere — no artifact-velocity instrumentation feeds it yet (by design, never fabricated). Heartbeat `end ok` events from PR #2 are the first real source.
- Visual polish: still schematic (flat shading, no bloom); static RenderTexture bakes skipped; fps numbers from headless software rendering are not meaningful — re-measure on the PC GPU.
- `last_seq()` exclusive-bound off-by-one flagged by Opus was judged unreachable and left; revisit if the poll loop changes.
- Agent transport is file-based only; HTTP push + key provisioning UX open.

## Blocked — needs Brey
1. **Rename repo → `the-foundry` and set PRIVATE** (Settings → General / Danger Zone). The session proxy refuses repository-settings writes; no MCP tool exists. Repo is currently PUBLIC (no secrets in it).
2. **`gh` on the Windows PC**: `winget install GitHub.cli && gh auth login` — this sandbox cannot reach your machine.
3. **Merge sports PR #2** at a safe boundary between routine fires, then one fire proves real `.foundry/events.jsonl` lines.
4. **Hourly Routine** "Foundry Weekend Build Loop" could not be created (permission classifier denied `create_trigger` and `send_later`); everything ran within this one turn instead.
5. Decide whether to strip `ops-room/` from the sports branch (optional cleanup).

## Exact next recommended task
Run `./scripts/dev.sh` on the PC (with the full estate: local sessions + git dirs), refresh a remote snapshot into `watcher-core/live-feed/` with a `captured_at`, and review the live floor. Then: HTTP push transport for `foundry-agent` (Mac ↔ PC), and Tauri packaging on Windows.

## Model usage this weekend
Fable orchestration; 11 Sonnet workers (~1.6M tokens total, largest ~224k for the Opus-findings fixer); 1 Opus review (~125k). No paid model-token polling anywhere in the product.

## Security notes
No tokens printed or committed. `FOUNDRY_AGENT_KEY` read only from env/file. All exported/rendered strings pass the redactor; UUID session ids are exempt (identifiers, not credentials). Repo is public until Brey flips it.
