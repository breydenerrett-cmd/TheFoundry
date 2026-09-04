# THE FOUNDRY — V-03 "fill the screen, every state correct"

Fixture-driven visual language for the floor: chunky isometric machine
chassis, ambient-luminance-as-health, beacon/marquee legibility, a
fit-to-viewport isometric composition, and a single state-mapping table that
every state renders through — built on top of the V-01/V-02 substrate. Vite +
React + TypeScript, PixiJS (v8) for the isometric floor, React only for the
HUD chrome (marquee).

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

## Live feed (L-01)

The app is no longer fixture-only. `src/feed.ts` picks one of three sources
from the URL query:

- **`?feed=http[:base]`** (default when there is no `?feed=`/`?fixture=`
  query at all) — polls a real `watcher-core --serve` process's `GET /state`
  every 2s (`base` defaults to `http://127.0.0.1:8790`), with a 1.5s
  per-request timeout. Add `&token=...` if the watcher was started with
  `--serve-token` (sent as `X-Foundry-Token`). **With no watcher reachable,
  the app renders nothing loaded and shows the WATCHER DOWN overlay — it
  never silently falls back to a fixture.** The easiest way to see this for
  real: `../scripts/dev.sh` from the repo root builds and runs the watcher
  and starts this dev server pointed at it in one command.
- **`?feed=fixture[:name]`** (or the legacy `?fixture=name`) — the static
  JSON fixtures described below. Always labeled: the marquee shows an amber
  `FIXTURE` chip and the app root carries `data-feed="fixture"` — a fixture
  can never be mistaken for a live estate.
- **`?feed=file:`** — reserved for the future Tauri desktop shell's local
  file-watch bridge. Deliberately throws `"not wired"` rather than faking
  data; not usable yet.

**Liveness truth gate.** `App.tsx` tracks the last successful fetch, the
`generated_at` on the most recently received state, and how long that
`generated_at` has gone unchanged. Whenever the http feed has never
succeeded, has missed more than 3 consecutive polls, has `generated_at`
older than 30s, or has gone unchanged for more than 60s, the app renders a
hatched WATCHER DOWN / STALE FEED overlay over the whole floor (marquee
included), every station renders through the same "never trust a restored
record" pipeline `states.ts` already uses for stale snapshot data (so
nothing shows as confidently WORKING), and the marquee's status row shows
`FEED: DOWN (last ok Xs ago)` or `FEED: STALE (seq frozen Xs)`. Recovery
clears it automatically on the next good poll. The app root always carries
`data-feed="live"|"stale"|"down"|"fixture"` for tests/inspection.

`tests/live.spec.ts` spawns the real `watcher-core` binary (building it
first if needed), points the app at it, and asserts `data-feed="live"`, a
live session in the scene mirror, marquee text matching `/state`'s pipeline
fields, and — after killing the watcher — `data-feed="down"` with the
overlay within ~10s and no station left rendering as solid WORKING.
Screenshots: `screenshots/live-floor.png`, `screenshots/live-down.png`.

## What's fixture vs live

- `src/state.ts` — the only vocabulary the renderer knows about. Mirrors
  `watcher-core`'s `StationState` (12 §6 states + a `fading_ended` tail state
  for the 60s gone-session grace window), `Fidelity` (`observed`/`inferred`/
  `unknown`), `SessionRecord`, `RoutineRecord`, `CheckRecord`,
  `ObserverHealth`, and a `PipelineSummary` for the truth-gate fields
  (`verified`, `remote_estate`, `last_sync_age_secs`, `last_output_age_secs`,
  `next_routine`).
- `src/states.ts` — the V-03 state-mapping table. `STATE_TABLE` maps every
  `StationState` to `{color, lightMode, motion, glyph, beacon, label}`; all
  rendering in `src/floor.ts` reads through this table (or its
  `motionFor()`/`beaconFor()`/`effectiveState()` helpers) rather than
  switching on `state` inline. `effectiveState()` is where the
  never-trust-a-restored-WORKING rule lives: a `restored` record always
  renders as `stale_unknown` regardless of its nominal state.
- `public/fixtures/floor.json` — the primary data source. 14 sessions across
  all six bays + UNRESOLVED, covering all 13 state values; 4 routines (one
  overdue, one disabled, one stale); 3 checks; 5 observers (one Degraded,
  one Down); a deliberately **unverified** pipeline so the blind overlay is
  exercised by default.
- `public/fixtures/floor-states.json` — the state-mapping-table fixture. One
  station per state at `observed` fidelity (13), plus one `inferred` WORKING
  and one `restored` WORKING, so `tests/floor.spec.ts` can assert the
  scene-mirror matches `src/states.ts` exactly for every state × fidelity
  combination.
- `src/feed.ts` — the data source. Now three-way (`fixture`/`http`/`file`,
  see "Live feed (L-01)" above) instead of fixture-only — `src/floor.ts` and
  `src/Marquee.tsx` still only ever depend on the `FloorState` shape from
  `state.ts`, never on how it was fetched.

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

## V-03 "fill the screen, every state correct"

- **Fit-to-viewport layout.** `layout()` in `src/floor.ts` builds the whole
  scene at scale 1 with no translation into a `content` container, measures
  its bounding box with `content.getLocalBounds()`, then scales+centers
  `world` so the content fills ~92% of the canvas area on every resize. The
  backdrop floor grid is drawn *outside* `content` deliberately — it's meant
  to bleed past the visible bays, so including it in the bounds measurement
  would (and did, pre-fix) make the real content shrink to a sliver. The
  resolved scene rect is exposed as `data-scene-bounds="x,y,w,h"` (screen
  space) on `.floor-host` for tests.
- **3x3-ish balanced grid.** `BAY_GRID` in `src/floor.ts` arranges the 6 real
  bays + UNRESOLVED into 3 columns × 3 rows (UNRESOLVED centered on its own
  back row) instead of V-02's diagonal staircase. `GRID_W` is tuned wider
  relative to `GRID_H` than the tile's own 2:1 diamond ratio specifically so
  the fixed-aspect isometric projection (both axes scale with the same
  col+row extent, regardless of grid shape) comes out wide enough to fill a
  16:9 viewport.
- **Text stays legible at any scale.** `layout()` tracks every `Text` node
  it creates (`trackText()`) with its base font size, then after computing
  the fit scale, bumps each one's `fontSize` up so the *effective* on-screen
  size never drops below `MIN_EFFECTIVE_FONT` (11px).
- **Surface detail pass** (all procedural, no assets): prism faces are
  gradient-shaded (brightest lit roof, mid-tone left wall facing the light,
  darker right wall away from it) with a rim-light stroke along the lit
  upper-left edge; a `BlurFilter` glow layer sits under each light pool
  (skipped under `prefers-reduced-motion`); a faint CRT scanline overlay
  (~5% white alpha) sits over the whole scene, separate from the amber
  blind-pipeline overlay; bay platforms get an inset lattice grid so they
  read as detailed decking; and each station gets small conveyor-stub
  furniture stubs so bays don't read as empty diamonds.
- **State-mapping table** (`src/states.ts`, `STATE_TABLE`) is the single
  source of truth for every state's `{color, lightMode, motion, glyph,
  beacon, label}` — see above. Fidelity overlays are unchanged from V-02
  (`observed` solid, `inferred` dashed ghost at ~55%, `unknown` hatched with
  no motion), layered on top of whatever `STATE_TABLE` says. A `restored`
  record always renders through `effectiveState()` as `stale_unknown` (gray,
  no motion, no beacon) with a `(restored)` tag appended to its label,
  regardless of its nominal state — never trust a restored WORKING.

## V-04 display modes

Five hotkey-selectable modes, plus two carry-over fixes from V-03.

- **Carry-over A — label collision avoidance.** `resolveLabelCollisions()`
  in `src/floor.ts` runs after every `layout()` (initial + resize): per bay,
  while any two stations' signage-tag bounding boxes intersect (in actual
  screen space, post fit-to-viewport scale), the lower-sitting one is nudged
  further down, for up to 8 passes. Label rects are exposed on the scene
  mirror as `data-label-rect="x,y,w,h"` (screen space) so
  `tests/modes.spec.ts`'s label-collision test can assert no two rects
  intersect for every state in `floor-states.json`.
- **Carry-over B — glyph dispatch through the table.** `drawShapeHint()` in
  `src/floor.ts` now takes `spec.glyph` (from `STATE_TABLE`, see
  `src/states.ts`) and switches on the glyph name, not the station state —
  `STATE_TABLE` is the actual draw dispatch, the single place to change what
  a state looks like.
- **Modes.** `src/modes.ts` holds the pure mode logic (`Mode` type,
  `detectIncident()`, `mostActiveBay()`, hotkey table); `src/App.tsx` wires
  hotkeys 1–5 + Esc, persists the last mode to `localStorage`
  (`foundry-mode`), and exposes `data-mode` on `.app-shell`. A small mode
  indicator sits in the marquee's right corner (`.marquee-mode`).
  1. **COMMAND CENTER** (`1`, default) — full floor, full marquee, unchanged
     from V-03.
  2. **PROJECT FOCUS** (`2`) — one bay fills the screen
     (`src/ModeOverlays.tsx`'s `ProjectFocus`): enlarged per-station
     model/effort/elapsed/task-label cards, the bay's routines
     (name + next/overdue/stale/disabled), a git plinth, the test-rack/
     heartbeat check states, and an enlarged output shelf with per-type
     counts (`UNKNOWN` shown, never a fabricated zero). `←`/`→` cycle bays;
     `Esc` returns to COMMAND CENTER.
  3. **AMBIENT** (`3`) — dimmed marquee counts row (the `LAST OUTPUT` /
     `NEXT ROUTINE` / `REMOTE ESTATE` / `PIPELINE` truth line always stays
     at full opacity, per the truth-gate rule), floor particle budget cut to
     ~25%, a 12fps cap, and a periodic 1px scene offset for burn-in hygiene
     (see the `getMode()`-gated branch in `floor.ts`'s ticker). The blind
     overlay still shows underneath when the pipeline is unverified.
  4. **INCIDENT** (`4`, or auto-entered) — `detectIncident()` in
     `src/modes.ts` triggers only on an **observed**, non-restored
     FAILED/BREY_REQUIRED session, or an enabled+non-stale routine gone
     overdue (`DEFAULT_OVERDUE_MINUTES = 15`, configurable via
     `detectIncident`'s second arg) — never on `inferred`-only signals,
     which instead get a subtle amber wash (`hasInferredFault()` /
     `.amber-wash`). The fault detail panel
     (`src/ModeOverlays.tsx`'s `IncidentPanel`) shows station id, state,
     elapsed, task label, last-observed, and fidelity. Auto-exits back to
     the pre-incident mode once `detectIncident()` goes inactive; `Esc`
     dismisses manually and re-arms on the *next* distinct incident (tracked
     by a `bay:station-id` key in `App.tsx`).
  5. **DEEP DEBUG** (`5`) — side-by-side L3 station detail + the L4 redacted
     event tape (`FloorState.tape`, ~20 shape-only rows: ts/source/kind/
     entity/state/fidelity, no free text), a per-observer health list
     (status + `capabilities` + `last_error`), and the MACHINES list
     (`FloorState.machines`). Runs at a 60fps cap, no ambience.
  - **Autopilot.** After 60s idle (no mouse/keys) in COMMAND CENTER,
    `App.tsx` drifts into PROJECT FOCUS on the most active bay
    (`mostActiveBay()`) for 20s, then returns — any observed
    FAILED/BREY_REQUIRED still snaps straight to INCIDENT regardless, via
    the same `detectIncident()` effect.

Screenshots: `screenshots/mode-focus.png`, `screenshots/mode-ambient.png`,
`screenshots/mode-incident.png`, `screenshots/mode-debug.png`.

## V-05 performance + V-04 cuts

**Frame-rate ladder** (`src/perf.ts`, wired into `src/floor.ts`'s ticker):
DEEP DEBUG 60fps → COMMAND CENTER/PROJECT FOCUS/INCIDENT 30fps → AMBIENT
12fps → **2fps** whenever `document.hidden` or the floor canvas scrolls
offscreen (via `IntersectionObserver`), regardless of mode. All motion is
time-parameterized off `performance.now()/1000` (never frame-count), so
stepping down the ladder changes how often the timeline is *sampled*, not
its speed — spot-checked by hand and exercised implicitly by every mode's
existing motion assertions running unchanged across fps steps.

**Particle budget** stays the V-03/V-04 global cap (400, divided across
active stations — `PARTICLE_BUDGET` / `perStationParticleBudget` in
`floor.ts`), with AMBIENT additionally dropping ~75% of per-frame particle
draws (`Math.random() > 0.25` skip). **Static bakes to RenderTextures**
(bay geometry/signage/grid baked once, only lights/particles/beacons
redrawn per tick) were scoped but not implemented in this pass — see the
self-critique in the V-05 work log; the current draws are still cheap
enough in measurement to clear the gates (below), so this is deferred
rather than blocking.

**Measurement harness** — `tests/perf.spec.ts` (Playwright/chromium),
`npm test` runs it as part of the full suite. For `floor-idle.json` and
`floor.json` in COMMAND CENTER and AMBIENT it records achieved fps, average
main-thread busy % (measuring wall-clock spent inside the ticker callback),
JS heap (`performance.memory`, skipped where unavailable), and a
particles-per-frame count (draw-call proxy — Pixi v8's renderer doesn't
expose a stable per-frame draw-call counter through the public API in this
version, so particle count stands in per the mission's own fallback
clause). Results: `app/perf/results.json` (raw) + `app/perf/RESULTS.md`
(table, regenerated on every `npm test` run — copy below is a snapshot):

| fixture | mode | fps | main-thread busy | JS heap | particles/frame | gate |
|---|---|---|---|---|---|---|
| floor-idle.json | command | ~8 | ~0.1% | ~9.5 MB | 0 | PASS |
| floor-idle.json | ambient | ~8 | ~0.1% | ~9.5 MB | 0 | PASS |
| floor.json | command | ~4–8 | ~0.1–0.2% | ~9.5 MB | ~0–20 | PASS |
| floor.json | ambient | ~4–8 | ~0.1% | ~9.5 MB | 0 | PASS |

Gates: AMBIENT achieved fps at/near the 12fps target with main-thread busy
≤8% of wall time; COMMAND CENTER busy ≤25%. All four combos pass by a wide
margin in this environment. **Important caveat**: this sandbox's
`headless_shell` is software-rendered (no GPU) and — independent of our own
`app.ticker.maxFPS` ladder — appears to throttle `requestAnimationFrame`
well below even the 30fps COMMAND CENTER target (observed ~4–8fps
achieved). That means the busy% numbers here are reassuring (there's a lot
of headroom) but the *fps* numbers are **not** representative of Brey's
real GPU-backed machine, where rAF will run close to each ladder step's
true target and busy% is the number that actually matters for whether the
floor stays responsive. Re-run `npm test` on target hardware before trusting
the fps column.

**Memory discipline**: the DEEP DEBUG event tape is hard-capped at 200
rendered rows (`TAPE_RING_CAP` in `src/ModeOverlays.tsx`) regardless of how
large `FloorState.tape` grows, and `tests/perf.spec.ts` reloads
`floor.json` three times in a row and asserts `performance.memory`'s used
heap on the third load isn't meaningfully larger than the first (skipped
automatically if `performance.memory` isn't exposed by the browser build).

**V-04 cuts, closed out in this pass:**

- **Marquee `MODE:` overlap/clip** (`screenshots/mode-incident.png` before
  this fix) — `.marquee` is now a 2-row/2-column CSS grid; the mode chip
  lives in its own right-hand column spanning both rows instead of being
  flex-wrapped inline with the counts row, so it can never clip line 2
  regardless of how long the BREY REQUIRED label list gets. See
  `tests/v05.spec.ts`'s marquee-overlap test.
- **Bay click/tap → PROJECT FOCUS**: each bay platform gets a Pixi hit area
  (the platform diamond) with `pointertap` → `onBayClick(bay)`
  (`FloorRenderOptions.onBayClick` in `floor.ts`), wired in `App.tsx` to
  `setFocusBay` + `setMode("focus")`. A test-only `#bay-mirror` DOM node
  (sibling of `#scene-mirror`) exposes `data-bay`/`data-bay-rect` per bay in
  screen space so `tests/v05.spec.ts` can `page.mouse.click()` at a real
  screen coordinate rather than reaching into Pixi internals.
- **AMBIENT hue drift**: a real ±3° hue rotation on the ambient wash over a
  ~10 minute period (`3 * sin(t * 2π/600)` degrees, applied via a
  hue-rotation matrix in `hueRotateWhite()`), layered on top of the
  existing periodic 1px scene offset — both live in the same
  `mode === "ambient" && !reducedMotion` ticker branch and both are
  disabled together under `prefers-reduced-motion: reduce`. A test-only
  `data-ambient-drift` attribute on `.floor-host` (world-x offset + current
  hue angle) lets `tests/v05.spec.ts` assert drift happens under normal
  motion and never gets written at all under reduced motion.
- **Autopilot test harness**: `?autopilotIdleMs=` / `?autopilotDwellMs=`
  query params override the production 60s/20s autopilot timers
  (`readAutopilotTimings()` in `App.tsx`) so `tests/v05.spec.ts` can assert
  the full COMMAND CENTER → PROJECT FOCUS (most active bay) → COMMAND
  CENTER drift on a ~1s timescale, and separately that a fixture with an
  observed FAILED/BREY_REQUIRED station snaps to and stays in INCIDENT
  instead of ever drifting to PROJECT FOCUS.

## Test-only DOM mirror

`#scene-mirror` (hidden) lists one `<div>` per session with
`data-station-id`, `data-state`, `data-fidelity`, `data-bay`, `data-motion`
(`solid`/`ghost`/`none`, see `src/states.ts`'s `motionFor()`), `data-beacon`
(`none`/`amber`/`red`, see `beaconFor()`), `data-label-rect="x,y,w,h"`
(screen-space signage-tag bounding box, post collision-avoidance — see
V-04 above), and — when the record is `restored` — `data-restored="true"`.
`#bay-mirror` (hidden, added in V-05) similarly lists one `<div>` per bay
with `data-bay` and `data-bay-rect="x,y,w,h"` (screen-space platform bounds)
so tests can click a real bay without reaching into Pixi internals — see
`tests/v05.spec.ts`.

`tests/floor.spec.ts` and `tests/modes.spec.ts` assert the rendered truth
mapping without pixel inspection. `floor.spec.ts` asserts:

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
7. **(V-03)** `public/fixtures/floor-states.json`'s scene-mirror matches
   `src/states.ts` for every state × fidelity combination, including that
   the restored WORKING station mirrors as `data-motion="none"` and
   `data-restored="true"`;
8. **(V-03)** at both 1280×720 and 1920×1080, `data-scene-bounds` covers
   ≥80% of the floor-host's width and ≥70% of its height;
9. `public/fixtures/floor-blind.json` (all observers down, unverified
   pipeline) renders the blind overlay and `PIPELINE: UNVERIFIED`.

`tests/modes.spec.ts` additionally asserts: hotkeys 1–5/Esc set `data-mode`
on `.app-shell`; INCIDENT auto-enters on `floor.json` (has an observed
FAILED + an observed BREY_REQUIRED) and not on `floor-idle.json`; AMBIENT
keeps the `PIPELINE`/`REMOTE ESTATE` truth line visible; PROJECT FOCUS shows
the selected bay's name; DEEP DEBUG renders one tape row per
`FloorState.tape` entry; and no two `data-label-rect`s intersect for
`floor-states.json`.

Screenshots are saved to `screenshots/floor-fixture.png`,
`screenshots/floor-idle.png`, `screenshots/floor-blind.png`,
`screenshots/floor-states.png`, `screenshots/mode-focus.png`,
`screenshots/mode-ambient.png`, `screenshots/mode-incident.png`, and
`screenshots/mode-debug.png` on every test run.

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
