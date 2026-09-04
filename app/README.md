# THE FOUNDRY — V-01 "Substrate" (visual foundation)

Fixture-driven static visual foundation for the floor. Vite + React + TypeScript,
PixiJS (v8) for the isometric floor, React only for the HUD chrome (marquee).

## Run it

```
npm install
npm run dev        # http://localhost:5173, live-reload
npm run build      # tsc -b && vite build -> dist/
npm run preview    # serve the production build at :4173
npm test           # Playwright, chromium headless_shell at /opt/pw-browsers
```

`npm run build` and `npm test` are both standalone — no Tauri build is
required or attempted in this environment (see "Tauri scaffold" below).

## What's fixture vs live

- `src/state.ts` — the only vocabulary the renderer knows about. Mirrors
  `watcher-core`'s `StationState` (12 §6 states + a `fading_ended` tail state
  for the 60s gone-session grace window), `Fidelity` (`observed`/`inferred`/
  `unknown`), `SessionRecord`, `RoutineRecord`, `CheckRecord`,
  `ObserverHealth`, and a `PipelineSummary` for the truth-gate fields
  (`verified`, `remote_estate`, `last_sync_age_secs`, `last_output_age_secs`,
  `next_routine`).
- `public/fixtures/floor.json` — the only data source today. 14 sessions
  across all six bays + UNRESOLVED, covering all 13 state values; 4 routines
  (one overdue, one disabled, one stale); 3 checks; 5 observers (one
  Degraded, one Down); a deliberately **unverified** pipeline so the blind
  overlay is exercised by default.
- `src/feed.ts` — the loader. Reads `?fixture=<name>.json` from the URL query
  (sandboxed to `public/fixtures/`) or defaults to `floor.json`. This is the
  **only** file that will change when a live `/state` WebSocket or Tauri
  event stream replaces the fixture — `src/floor.ts` and `src/Marquee.tsx`
  only ever depend on the `FloorState` shape from `state.ts`, never on how it
  was fetched.

No other runtime network calls are made.

## Truth rules the renderer enforces

Ported directly from `watcher-core`'s truth gate (see `../PHASE3_REPORT.md`):

- **Absence is UNKNOWN, never healthy.** `Fidelity.unknown` always renders
  hatched, never as a clean/confident state. `restored` sessions render with
  STALE treatment regardless of their nominal state, plus a `(restored)`
  cue in the scene mirror.
- **A blind pipeline never shows a clean floor.** Whenever
  `pipeline.verified` is false or `remote_estate !== "live"`, a scanline
  overlay tints the whole floor and the marquee gets a `marquee-blind` class
  — see `drawOverlay()` in `src/floor.ts`.
- **`UNRESOLVED` is never guessed into a real bay.** It renders dimmer, with
  a hatched header reading "(not guessed)", per `foundry.bays.toml`'s
  resolution-order comment.
- **Missing instrumentation on the output shelf is not a zero.** A `null`
  slot count renders as a hatched empty token; only `Some(0)` renders as a
  confident, filled-but-empty zero.
- **Every state is distinguishable without color** — shape and motion carry
  the state distinction too (circle/square/diamond/triangle families, plus
  pulse/flicker/rotation/ghosting), for colorblind-safety.

## Test-only DOM mirror

`#scene-mirror` (hidden) lists one `<div>` per session with
`data-station-id`, `data-state`, `data-fidelity`, `data-bay` attributes, so
`tests/floor.spec.ts` can assert the rendered truth mapping without pixel
inspection. `tests/floor.spec.ts` asserts:

1. every fixture session appears in the mirror with its exact state token;
2. the unverified-pipeline fixture produces the blind overlay class + text;
3. the marquee's `N BREY REQUIRED` count matches the fixture.

A screenshot of the fixture floor is saved to `screenshots/floor-fixture.png`
on every test run.

## Tauri scaffold

`src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and `src-tauri/src/main.rs`
are present as a minimal starting point for the eventual desktop shell, but
**were not built or run in this sandbox** — it has no `webkit2gtk`, which
`tauri-build`/`cargo tauri build` require on Linux. The web build
(`npm run build` + `npm run preview`) is what's actually verified here and is
what CI/tests depend on. Wiring the Tauri shell (real `cargo tauri dev`,
native window, IPC to a live `watcher-core` process) is future work, not part
of V-01.

## Fonts / assets

System/monospace font stack only (`"Berkeley Mono", "JetBrains Mono",
ui-monospace, "SF Mono", Menlo, Consolas, "Courier New", monospace` — see
`src/theme.ts`'s `FONT_STACK`). No Google Fonts, no proprietary game/broadcast
assets. All floor visuals are drawn procedurally with Pixi `Graphics`.
