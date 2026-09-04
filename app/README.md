# THE FOUNDRY — V-02 "3-second glance" (visual language)

Fixture-driven visual language for the floor: chunky isometric machine
chassis, ambient-luminance-as-health, and beacon/marquee legibility, built on
top of the V-01 substrate. Vite + React + TypeScript, PixiJS (v8) for the
isometric floor, React only for the HUD chrome (marquee).

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

## V-02 visual language

The "3-second glance" pass: stations are no longer tiny primitives, and the
floor as a whole is legible from across a room without reading any single
station.

- **Chunky isometric chassis.** Each station is an extruded prism (~3x the
  V-01 primitive size) with a light pool glowing underneath, a wide-tracked
  uppercase signage plate, and a small colorblind-safe shape glyph on its
  roof (the V-01 circle/square/diamond/triangle family is preserved as that
  glyph). Bays got low walls (a shallow skirt beneath the platform diamond)
  and a floor grid glow instead of a flat dark background.
- **Ambient luminance = health (the #1 glance signal).** `computeAmbient()`
  in `src/floor.ts` derives a 0..1 level from the fraction of sessions that
  are WORKING/THINKING: a busy floor gets a warm lit wash, an idle floor is
  dim and breathes slowly (`AMBIENT_LIT`/`AMBIENT_DIM` in `src/theme.ts`).
  The level is exposed as `data-ambient` on `.floor-host` for tests.
- **Motion density, fidelity-gated.** WORKING gets a rhythmic tool-head
  stroke + amber spark particles; THINKING gets a slow blue breathing pulse
  + upward drift particles; SPECIALIST (Opus) gets a violet chamber + slow
  plume. `inferred` fidelity never renders these solid — it gets the same
  motion class as a dashed ghost outline at ~50% alpha instead
  (`motionFor()` returns `"ghost"`). `unknown` fidelity is always static
  hatched (`motionFor()` returns `"none"`) regardless of state. A global
  particle budget (~400, `PARTICLE_BUDGET`) is divided across active
  stations. All motion is driven by `performance.now()`, never frame count,
  capped at 30fps, and fully suppressed under `prefers-reduced-motion`
  (colors/beacons/labels stay static-visible).
- **Beacons.** BREY_REQUIRED gets the tallest pole, the hardest pulse, a
  raised flag, and its station labels are named directly in the marquee's
  `N BREY REQUIRED` line. FAILED gets a rotating beacon sweep plus a
  floor-wide amber wash. HUNG gets alpha-flicker, a live climbing
  `mm:ss`/`h:mm:ss` elapsed label, and a frozen billet on a conveyor stub.
  STALE/UNKNOWN gets desaturation (gray chassis), a scanline-alpha tear, and
  a "LAST SYNC —" tag.
- **Marquee.** Fixed-height, ≥20px counts row in this order: `N BREY
  REQUIRED` (amber, station labels appended) · `N FAILED` · `N HUNG` ·
  `N WORKING` · `N WAITING` (agent+system+blocked) · `N STALE` · `N OPUS`,
  followed by the existing status row (`LAST OUTPUT`, `NEXT ROUTINE`,
  `REMOTE ESTATE`, `LAST SYNC`, `PIPELINE`). Zero counts render dim
  (`.marquee-dim`); non-zero attention counts (FAILED/HUNG/STALE) render
  bright amber (`.marquee-attention`); UNKNOWN-state counts are always
  computed from real fixture data and rendered, never silently omitted as a
  fabricated zero.

## Test-only DOM mirror

`#scene-mirror` (hidden) lists one `<div>` per session with
`data-station-id`, `data-state`, `data-fidelity`, `data-bay`, `data-motion`
(`solid`/`ghost`/`none`, see `motionFor()`) attributes, so
`tests/floor.spec.ts` can assert the rendered truth mapping without pixel
inspection. `tests/floor.spec.ts` asserts:

1. every fixture session appears in the mirror with its exact state token;
2. the unverified-pipeline fixture produces the blind overlay class + text;
3. the marquee's `N BREY REQUIRED` count matches the fixture;
4. the marquee counts row renders in the required order with correct
   FAILED/HUNG/WORKING/WAITING/STALE/OPUS counts;
5. an `inferred`-fidelity WORKING station mirrors as `data-motion="ghost"`,
   and an `unknown`-fidelity station mirrors as `data-motion="none"`;
6. `public/fixtures/floor-idle.json` (all IDLE, verified pipeline, live
   remote) renders no blind overlay and a lower `data-ambient` than the busy
   default fixture;
7. `public/fixtures/floor-blind.json` (all observers down, unverified
   pipeline) renders the blind overlay and `PIPELINE: UNVERIFIED`.

Screenshots are saved to `screenshots/floor-fixture.png`,
`screenshots/floor-idle.png` and `screenshots/floor-blind.png` on every test
run.

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
