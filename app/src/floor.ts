// Pixi floor renderer — isometric 2:1 fake-3D dark industrial floor.
// Pure rendering module: takes a FloorState snapshot and draws it. No network,
// no state mutation beyond Pixi's own scene graph. See feed.ts for data.

import { Application, BlurFilter, Container, Graphics, Text, TextStyle } from "pixi.js";
import type { BayName, Fidelity, FloorState, SessionRecord, StationState } from "./state";
import { BAYS, isOpusModel } from "./state";
import { AMBIENT_DIM, AMBIENT_LIT, COLORS, FONT_STACK, STATE_LABEL } from "./theme";
import { STATE_TABLE, beaconFor, effectiveState, motionFor as motionForTable } from "./states";

/** Minimum effective (post-scale) font size, per V-03 §1 legibility rule. */
const MIN_EFFECTIVE_FONT = 11;

const TILE_W = 220;
const TILE_H = 110;
// Grid spacing is wider than the platform diamond itself so adjacent bays'
// signage, stations and output shelves have breathing room.
// Horizontal spacing is wider relative to vertical than the tile's own 2:1
// diamond ratio so the 3x3-ish bay grid reads as a wide isometric deck that
// naturally fills a 16:9-ish viewport (see the fit-to-viewport comment on
// `layout()` — this ratio, not the layout shape, determines the content's
// aspect since both axes scale with the same col+row extent).
const GRID_W = TILE_W * 3.0;
const GRID_H = TILE_H * 2.6;

// Chassis footprint — ~3x the V-01 primitive size (was radius 7-9).
const ST_W = 26; // half-width of the chassis top diamond
const ST_H = 13; // half-height of the chassis top diamond
const ST_LIFT = 22; // extrusion height of the prism walls

const MOTION_STATES: ReadonlySet<StationState> = new Set(["working", "thinking", "specialist"]);
const PARTICLE_BUDGET = 400;

// 3x3-ish isometric grid for the 7 bays (6 real + UNRESOLVED) so the
// composition reads as a balanced deck rather than a diagonal staircase.
// UNRESOLVED sits centered on its own row, dimmer, at the back.
const BAY_GRID: Record<BayName, { col: number; row: number }> = {
  "SPORTS LAB": { col: 0, row: 0 },
  "AI BUSINESS COMPLEX": { col: 1, row: 0 },
  SERVERFORGE: { col: 2, row: 0 },
  "MUSIC LAB": { col: 0, row: 1 },
  EXPERIMENTS: { col: 1, row: 1 },
  "PERSONAL/MISC": { col: 2, row: 1 },
  UNRESOLVED: { col: 1, row: 2 },
};

function isoProject(col: number, row: number): { x: number; y: number } {
  return {
    x: (col - row) * (GRID_W / 2),
    y: (col + row) * (GRID_H / 2),
  };
}

function hatchPattern(g: Graphics, w: number, h: number, color: number, alpha = 0.35): void {
  const step = 10;
  g.setStrokeStyle({ width: 1, color, alpha });
  for (let x = -h; x < w; x += step) {
    g.moveTo(x, 0);
    g.lineTo(x + h, h);
  }
  g.stroke();
}

function dashedCircle(g: Graphics, radius: number, color: number, alpha = 0.9): void {
  const segments = 16;
  g.setStrokeStyle({ width: 2, color, alpha });
  for (let i = 0; i < segments; i++) {
    if (i % 2 === 1) continue;
    const a0 = (i / segments) * Math.PI * 2;
    const a1 = ((i + 0.6) / segments) * Math.PI * 2;
    g.moveTo(Math.cos(a0) * radius, Math.sin(a0) * radius);
    g.lineTo(Math.cos(a1) * radius, Math.sin(a1) * radius);
  }
  g.stroke();
}

function dashedDiamond(g: Graphics, w: number, h: number, color: number, alpha = 0.9): void {
  const pts = [
    [0, -h],
    [w, 0],
    [0, h],
    [-w, 0],
  ];
  g.setStrokeStyle({ width: 2, color, alpha });
  for (let i = 0; i < 4; i++) {
    const [ax, ay] = pts[i];
    const [bx, by] = pts[(i + 1) % 4];
    const dashes = 5;
    for (let d = 0; d < dashes; d += 2) {
      const t0 = d / dashes;
      const t1 = (d + 1) / dashes;
      g.moveTo(ax + (bx - ax) * t0, ay + (by - ay) * t0);
      g.lineTo(ax + (bx - ax) * t1, ay + (by - ay) * t1);
    }
  }
  g.stroke();
}

/** Motion class exposed on the scene mirror per V-02 §3: `solid` for
 *  observed WORKING/THINKING/SPECIALIST, `ghost` for the same states at
 *  `inferred` fidelity (dashed ghost outline, 50% alpha, never solid),
 *  `none` for everything else including all `unknown` fidelity. */
export function motionFor(
  state: StationState,
  fidelity: Fidelity,
  restored?: boolean
): "solid" | "ghost" | "none" {
  return motionForTable(state, fidelity, restored);
}

/** Ambient luminance = health: 0..1 derived from the fraction of sessions
 *  that are WORKING or THINKING. This is the #1 glance signal — a busy
 *  floor is warm and lit, an idle floor is dim with a slow breathing wash. */
export function computeAmbient(sessions: SessionRecord[]): number {
  if (sessions.length === 0) return 0.2;
  const active = sessions.filter((s) => s.state === "working" || s.state === "thinking").length;
  const frac = active / sessions.length;
  return Math.max(0.18, Math.min(1, 0.18 + frac * 1.1));
}

interface Particle {
  g: Graphics;
  dx: number;
  dy: number;
  seed: number;
}

interface StationSprite {
  container: Container;
  chassis: Graphics;
  beacon: Graphics | null;
  label: Text | null;
  fidelityOverlay: Graphics;
  particleLayer: Graphics;
  record: SessionRecord;
  phase: number;
  elapsedBase: number;
}

export interface FloorHandle {
  destroy: () => void;
}

export function mountFloor(canvasHost: HTMLDivElement, state: FloorState): Promise<FloorHandle> {
  const app = new Application();
  const reducedMotion = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;

  const ambient = computeAmbient(state.sessions);
  canvasHost.dataset.ambient = ambient.toFixed(2);

  return app
    .init({
      resizeTo: canvasHost,
      backgroundColor: COLORS.bg,
      antialias: true,
      preference: "webgl",
    })
    .then(() => {
      canvasHost.appendChild(app.canvas);

      const world = new Container();
      app.stage.addChild(world);

      const ambientWash = new Graphics();
      app.stage.addChild(ambientWash);

      const overlay = new Graphics();
      app.stage.addChild(overlay);

      const stations: StationSprite[] = [];
      const activeCount = state.sessions.filter(
        (s) => (s.state === "working" || s.state === "thinking" || s.state === "specialist") &&
          s.fidelity !== "unknown"
      ).length;
      const perStationParticleBudget = Math.max(
        2,
        Math.min(40, Math.floor(PARTICLE_BUDGET / Math.max(1, activeCount)))
      );

      // Text nodes created during layout, tracked so their font size can be
      // bumped back up to MIN_EFFECTIVE_FONT post-scale-down (V-03 §1: keep
      // text legible at min 11px *effective*, i.e. after `world.scale`).
      let scaledTexts: { text: Text; base: number }[] = [];
      function trackText(text: Text, base: number): Text {
        scaledTexts.push({ text, base });
        return text;
      }

      /** Fit-to-viewport: lay out the scene at scale 1 with no translation,
       *  measure its bounding box, then scale+center it to fill ~92% of the
       *  screen area (the canvas host already excludes the marquee via flex
       *  layout, so "available area" is simply app.screen). Runs on every
       *  resize. Exposes `data-scene-bounds="x,y,w,h"` in screen space. */
      function layout(): void {
        world.removeChildren();
        world.scale.set(1);
        world.x = 0;
        world.y = 0;
        scaledTexts = [];
        stations.length = 0;

        const { width, height } = app.screen;
        const shelfMap = state.output_shelf;

        // The backdrop grid is drawn wide on purpose (a floor that extends
        // past the visible bays) so it must NOT count toward the fit-to-
        // viewport bounding box below — only `content` (the actual bays)
        // does. Otherwise the huge backdrop would make the real content
        // shrink to a fraction of the screen, which was V-03's #1 defect.
        drawFloorGrid(world);
        const content = new Container();
        world.addChild(content);

        for (const bay of BAYS) {
          const grid = BAY_GRID[bay];
          const p = isoProject(grid.col, grid.row);
          const bayContainer = new Container();
          bayContainer.x = p.x;
          bayContainer.y = p.y;
          content.addChild(bayContainer);

          drawBayPlatform(bayContainer, bay);

          const sessions = state.sessions.filter((s) => s.bay === bay);
          const cols = 3;
          sessions.forEach((s, i) => {
            const sx = (i % cols) * 90 - 90;
            const sy = Math.floor(i / cols) * 56 - 18;
            const sprite = buildStation(s);
            sprite.container.x = sx;
            sprite.container.y = sy;
            bayContainer.addChild(sprite.container);
            stations.push(sprite);
          });

          drawOutputShelf(bayContainer, shelfMap[bay] ?? []);

          const effStates = sessions.map((s) => effectiveState(s.state, s.restored));
          const brey = effStates.filter((s) => s === "brey_required");
          const hasFault = effStates.some((s) => s === "failed" || s === "hung");
          if (brey.length > 0 || hasFault) {
            drawFaultBeacon(bayContainer, sessions);
          }
        }

        const bounds = content.getLocalBounds();
        const availW = width * 0.92;
        const availH = height * 0.92;
        const fitScale =
          bounds.width > 0 && bounds.height > 0
            ? Math.max(0.05, Math.min(availW / bounds.width, availH / bounds.height))
            : 1;
        world.scale.set(fitScale);
        world.x = width / 2 - (bounds.x + bounds.width / 2) * fitScale;
        world.y = height / 2 - (bounds.y + bounds.height / 2) * fitScale;

        const sceneX = bounds.x * fitScale + world.x;
        const sceneY = bounds.y * fitScale + world.y;
        canvasHost.dataset.sceneBounds = [
          sceneX.toFixed(1),
          sceneY.toFixed(1),
          (bounds.width * fitScale).toFixed(1),
          (bounds.height * fitScale).toFixed(1),
        ].join(",");

        // Keep chassis/text scaling with the scene, but never let text drop
        // below MIN_EFFECTIVE_FONT px on screen.
        for (const { text, base } of scaledTexts) {
          const needed = MIN_EFFECTIVE_FONT / fitScale;
          text.style.fontSize = Math.max(base, needed);
        }
      }

      function drawFloorGrid(container: Container): void {
        const grid = new Graphics();
        const glowColor = ambient > 0.55 ? AMBIENT_LIT : COLORS.bayEdge;
        const glowAlpha = 0.05 + ambient * 0.1;
        const span = 1400;
        const step = GRID_W / 2;
        grid.setStrokeStyle({ width: 1, color: glowColor, alpha: glowAlpha });
        for (let i = -6; i <= 6; i++) {
          grid.moveTo(-span, i * step);
          grid.lineTo(span, i * step);
        }
        container.addChildAt(grid, 0);
      }

      function drawBayPlatform(container: Container, bay: BayName): void {
        const isUnresolved = bay === "UNRESOLVED";
        const g = new Graphics();
        const w = TILE_W;
        const h = TILE_H;
        const fill = isUnresolved ? COLORS.bayFillDim : COLORS.bayFill;
        const edge = isUnresolved ? COLORS.bayEdgeUnresolved : COLORS.bayEdge;

        // Low walls: a shallow extruded skirt beneath the platform diamond
        // so bays read as raised decks, not flat dots-on-diamonds.
        const skirt = new Graphics();
        skirt.poly([-w / 2, 0, 0, h / 2, w / 2, 0, w / 2, 14, 0, h / 2 + 14, -w / 2, 14]);
        skirt.fill({ color: 0x05070a, alpha: 0.9 });
        skirt.stroke({ width: 1, color: edge, alpha: 0.5 });
        container.addChild(skirt);

        g.poly([0, -h / 2, w / 2, 0, 0, h / 2, -w / 2, 0]);
        g.fill({ color: fill, alpha: isUnresolved ? 0.55 : 0.9 });
        g.stroke({ width: 2, color: edge, alpha: 0.9 });
        container.addChild(g);

        // Inset floor-plate grid: a smaller diamond grid within the
        // platform so bays read as detailed decking, not a flat fill.
        const inset = new Graphics();
        const insetSteps = 4;
        inset.setStrokeStyle({ width: 1, color: edge, alpha: isUnresolved ? 0.15 : 0.28 });
        for (let i = 1; i < insetSteps; i++) {
          const t = i / insetSteps;
          inset.moveTo(-w / 2 + (w / 2) * t, -h / 2 + h * (0.5 - 0.5 * (1 - t)));
        }
        // Simple lattice of two diamond-parallel line families.
        for (let i = -insetSteps; i <= insetSteps; i++) {
          const t = i / insetSteps;
          inset.moveTo(t * (w / 2), t * (h / 2));
          inset.lineTo(t * (w / 2) + (w / 2) * (1 - Math.abs(t)), t * (h / 2) - (h / 2) * (1 - Math.abs(t)));
        }
        container.addChild(inset);

        if (isUnresolved) {
          const hatch = new Graphics();
          hatch.poly([0, -h / 2, w / 2, 0, 0, h / 2, -w / 2, 0]);
          hatch.fill({ color: 0, alpha: 0 });
          hatchPattern(hatch, w, h / 2, COLORS.bayEdgeUnresolved, 0.25);
          hatch.y = -h / 4;
          container.addChild(hatch);
        }

        // Signage plate: wide-tracked uppercase bay name on a backing panel.
        const plate = new Graphics();
        const plateW = Math.max(70, bay.length * 8);
        plate.roundRect(-plateW / 2, -h / 2 - 24, plateW, 16, 2);
        plate.fill({ color: 0x05070a, alpha: 0.85 });
        plate.stroke({ width: 1, color: edge, alpha: 0.8 });
        container.addChild(plate);

        const label = new Text({
          text: bay + (isUnresolved ? "  (not guessed)" : ""),
          style: new TextStyle({
            fontFamily: FONT_STACK,
            fontSize: 11,
            fill: isUnresolved ? COLORS.textDim : COLORS.text,
            letterSpacing: 2,
          }),
        });
        label.anchor.set(0.5, 0.5);
        label.y = -h / 2 - 16;
        container.addChild(trackText(label, 11));
      }

      function buildStation(record: SessionRecord): StationSprite {
        const container = new Container();
        // `restored` always forces STALE treatment — never trust a restored
        // WORKING. All rendering below reads the effective state, not the
        // raw nominal one, except the "(restored)" tag itself.
        const eff = effectiveState(record.state, record.restored);
        const spec = STATE_TABLE[eff];

        // Furniture: small non-empty-diamond detail per bay so stations
        // don't read as bare shapes — a conveyor stub feeding in and an
        // output ledge line, drawn low, behind the chassis.
        const furniture = new Graphics();
        furniture.rect(-ST_W - 6, 4, 10, 3);
        furniture.fill({ color: COLORS.bayEdge, alpha: 0.4 });
        furniture.rect(ST_W - 4, 4, 10, 3);
        furniture.fill({ color: COLORS.bayEdge, alpha: 0.4 });
        container.addChild(furniture);

        // Soft glow blur under the light pool — a separate low-cost layer,
        // disabled under reduced-motion (no need to keep animating a static
        // blurred blob when motion is off; it's still drawn once, just
        // without the filter cost, per the reduced-motion budget note).
        const lightPool = new Graphics();
        drawLightPool(lightPool, eff, spec);
        if (!reducedMotion) {
          lightPool.filters = [new BlurFilter({ strength: 6, quality: 2 })];
        }
        container.addChild(lightPool);

        const chassis = new Graphics();
        container.addChild(chassis);
        drawChassis(chassis, record, eff, spec);

        const particleLayer = new Graphics();
        container.addChild(particleLayer);

        const fidelityOverlay = new Graphics();
        container.addChild(fidelityOverlay);
        drawFidelityOverlay(fidelityOverlay, record);

        let beacon: Graphics | null = null;
        if (beaconFor(record.state, record.restored) !== "none") {
          beacon = new Graphics();
          container.addChild(beacon);
        }

        let label: Text | null = null;
        if (eff === "hung" || eff === "stale_unknown") {
          const restoredTag = record.restored ? " (restored)" : "";
          label = new Text({
            text: (eff === "hung" ? fmtElapsed(record.elapsed_secs) : "LAST SYNC —") + restoredTag,
            style: new TextStyle({
              fontFamily: FONT_STACK,
              fontSize: 9,
              fill: eff === "hung" ? COLORS.redOrange : COLORS.textDim,
              letterSpacing: 1,
            }),
          });
          label.anchor.set(0.5, 0);
          label.y = ST_LIFT + 8;
          container.addChild(trackText(label, 9));
        }

        // Signage plate under the chassis: wide-tracked uppercase state tag.
        const plate = new Graphics();
        const plateColor = spec.color;
        plate.roundRect(-30, ST_LIFT + (label ? 20 : 6), 60, 12, 2);
        plate.fill({ color: 0x05070a, alpha: 0.8 });
        plate.stroke({ width: 1, color: plateColor, alpha: 0.6 });
        container.addChild(plate);
        const tag = new Text({
          text: (STATE_LABEL[eff] ?? spec.label) + (record.restored ? " (restored)" : ""),
          style: new TextStyle({
            fontFamily: FONT_STACK,
            fontSize: 7,
            fill: plateColor,
            letterSpacing: 1.5,
          }),
        });
        tag.anchor.set(0.5, 0.5);
        tag.x = 0;
        tag.y = ST_LIFT + (label ? 26 : 12);
        container.addChild(trackText(tag, 7));

        return {
          container,
          chassis,
          beacon,
          label,
          fidelityOverlay,
          particleLayer,
          record,
          phase: Math.random() * Math.PI * 2,
          elapsedBase: record.elapsed_secs,
        };
      }

      function drawLightPool(g: Graphics, eff: StationState, spec: (typeof STATE_TABLE)[StationState]): void {
        if (spec.lightMode === "off") return;
        const color = spec.color;
        const rings = 3;
        for (let i = rings; i > 0; i--) {
          const r = (ST_W + 6) * (i / rings) * 1.4;
          g.ellipse(0, ST_LIFT * 0.55, r, r * 0.42);
          g.fill({ color, alpha: 0.05 + (0.05 * (rings - i + 1)) / rings });
        }
      }

      function drawChassis(
        g: Graphics,
        record: SessionRecord,
        eff: StationState,
        spec: (typeof STATE_TABLE)[StationState]
      ): void {
        g.clear();
        const desaturated = eff === "stale_unknown" || record.fidelity === "unknown";
        const baseColor = desaturated ? COLORS.gray : spec.color;
        const w = ST_W;
        const h = ST_H;
        const lift = ST_LIFT;
        const opus = isOpusModel(record.model_current ?? record.model) && eff === "specialist";

        // Bottom platform diamond (footprint).
        g.poly([0, 0, w, h / 2, 0, h, -w, h / 2]);
        g.fill({ color: 0x05070a, alpha: 0.7 });
        g.stroke({ width: 1, color: baseColor, alpha: 0.3 });

        // Gradient-shaded prism faces: lit top (roof), mid-tone left wall,
        // dark right wall — a fixed light source from the upper-left.
        // Left wall (mid face, faces the light).
        g.poly([-w, h / 2, 0, h, 0, h - lift, -w, h / 2 - lift]);
        g.fill({ color: baseColor, alpha: 0.5 });
        // Right wall (dark face, away from the light).
        g.poly([w, h / 2, 0, h, 0, h - lift, w, h / 2 - lift]);
        g.fill({ color: baseColor, alpha: 0.24 });
        // Back walls up to top diamond.
        g.poly([0, 0, w, h / 2, w, h / 2 - lift, 0, -lift]);
        g.fill({ color: baseColor, alpha: 0.4 });
        g.poly([0, 0, -w, h / 2, -w, h / 2 - lift, 0, -lift]);
        g.fill({ color: baseColor, alpha: 0.55 });

        // Top face (roof) — brightest, carries the state color.
        g.poly([0, -lift, w, h / 2 - lift, 0, h - lift, -w, h / 2 - lift]);
        g.fill({ color: baseColor, alpha: opus ? 0.85 : 0.95 });
        g.stroke({ width: 1.5, color: COLORS.white, alpha: 0.25 });

        // Rim light: a bright thin highlight along the roof's lit (upper
        // left) edge only, distinct from the all-around roof stroke above.
        g.setStrokeStyle({ width: 1.5, color: COLORS.white, alpha: 0.55 });
        g.moveTo(-w, h / 2 - lift);
        g.lineTo(0, -lift);
        g.stroke();

        // Specialist (opus) chamber walls: a violet enclosure ring.
        if (eff === "specialist") {
          g.setStrokeStyle({ width: 1.5, color: COLORS.violet, alpha: 0.7 });
          g.moveTo(-w * 0.6, h / 2 - lift * 1.35);
          g.lineTo(-w * 0.6, h / 2 - lift * 0.15);
          g.moveTo(w * 0.6, h / 2 - lift * 1.35);
          g.lineTo(w * 0.6, h / 2 - lift * 0.15);
          g.stroke();
        }

        // Non-color shape hint on the roof, colorblind-safe (matches V-01's
        // circle/square/diamond/triangle family per state).
        drawShapeHint(g, eff, h / 2 - lift);

        // HUNG: a frozen billet stalled on a conveyor stub.
        if (eff === "hung") {
          g.rect(-8, h / 2 - lift - 26, 16, 6);
          g.fill({ color: COLORS.gray, alpha: 0.8 });
          g.stroke({ width: 1, color: COLORS.redOrange, alpha: 0.9 });
          g.moveTo(-w * 0.7, h / 2 - lift - 22);
          g.lineTo(w * 0.7, h / 2 - lift - 22);
          g.stroke({ width: 2, color: COLORS.gray, alpha: 0.5 });
        }
      }

      function drawShapeHint(g: Graphics, state: StationState, cy: number): void {
        const c = COLORS.white;
        switch (state) {
          case "working":
          case "thinking":
            g.circle(0, cy, 4);
            g.fill({ color: c, alpha: 0.7 });
            break;
          case "specialist":
            g.poly([0, cy - 5, 5, cy, 0, cy + 5, -5, cy]);
            g.fill({ color: c, alpha: 0.7 });
            break;
          case "waiting_on_agent":
          case "waiting_on_system":
          case "blocked":
            g.rect(-4, cy - 4, 8, 8);
            g.stroke({ width: 1.5, color: c, alpha: 0.7 });
            if (state === "blocked") {
              g.moveTo(-3, cy - 3);
              g.lineTo(3, cy + 3);
              g.moveTo(3, cy - 3);
              g.lineTo(-3, cy + 3);
              g.stroke({ width: 1.5, color: c, alpha: 0.7 });
            }
            break;
          case "brey_required":
            g.poly([0, cy - 6, 5, cy - 2, 0, cy + 2]);
            g.fill({ color: c, alpha: 0.9 });
            break;
          case "failed":
            g.poly([0, cy - 5, 5, cy + 4, -5, cy + 4]);
            g.fill({ color: c, alpha: 0.85 });
            break;
          case "hung":
            g.poly([0, cy - 5, 5, cy + 4, -5, cy + 4]);
            g.stroke({ width: 1.5, color: c, alpha: 0.85 });
            break;
          case "idle":
            g.rect(-3, cy - 3, 6, 6);
            g.fill({ color: c, alpha: 0.4 });
            break;
          case "completed":
            g.circle(0, cy, 4);
            g.stroke({ width: 1.5, color: c, alpha: 0.9 });
            break;
          case "stale_unknown":
            hatchPattern(g, 10, 10, c, 0.5);
            break;
          case "fading_ended":
            g.circle(0, cy, 4);
            g.fill({ color: c, alpha: 0.2 });
            break;
        }
      }

      function drawFidelityOverlay(g: Graphics, record: SessionRecord): void {
        g.clear();
        if (record.fidelity === "inferred") {
          dashedDiamond(g, ST_W + 6, ST_H + 3 - ST_LIFT / 6, COLORS.textDim, 0.6);
          g.y = -ST_LIFT / 2;
        } else if (record.fidelity === "unknown") {
          hatchPattern(g, 2 * ST_W, 2 * ST_W, COLORS.textDim, 0.35);
          g.x = -ST_W;
          g.y = -ST_LIFT - ST_W * 0.5;
        }
      }

      function drawOutputShelf(
        container: Container,
        slots: FloorState["output_shelf"][BayName]
      ): void {
        const shelf = new Container();
        shelf.y = TILE_H / 2 + 14;
        container.addChild(shelf);

        const line = new Graphics();
        line.moveTo(-TILE_W / 2 + 10, 0);
        line.lineTo(TILE_W / 2 - 10, 0);
        line.stroke({ width: 1, color: COLORS.bayEdge, alpha: 0.6 });
        shelf.addChild(line);

        slots.forEach((slot, i) => {
          const x = -TILE_W / 2 + 20 + i * 34;
          const token = new Graphics();
          token.y = 12;
          token.x = x;
          if (slot.count === null) {
            token.rect(-6, -6, 12, 12);
            hatchPattern(token, 12, 12, COLORS.textDim, 0.5);
            token.stroke({ width: 1, color: COLORS.textDim, alpha: 0.6 });
          } else {
            token.rect(-6, -6, 12, 12);
            token.fill({ color: COLORS.green, alpha: slot.count > 0 ? 0.85 : 0.15 });
            token.stroke({ width: 1, color: COLORS.green, alpha: 0.8 });
          }
          shelf.addChild(token);

          const label = new Text({
            text: slot.count === null ? "?" : String(slot.count),
            style: new TextStyle({
              fontFamily: FONT_STACK,
              fontSize: 9,
              fill: COLORS.textDim,
            }),
          });
          label.anchor.set(0.5, 0);
          label.x = x;
          label.y = 20;
          shelf.addChild(label);
        });
      }

      function drawFaultBeacon(container: Container, sessions: SessionRecord[]): void {
        const brey = sessions.some((s) => effectiveState(s.state, s.restored) === "brey_required");
        const failed = sessions.some((s) => effectiveState(s.state, s.restored) === "failed");
        const hung = sessions.some((s) => effectiveState(s.state, s.restored) === "hung");
        const beacon = new Graphics();
        const height = brey ? 60 : failed ? 46 : 38;
        beacon.y = -TILE_H / 2 - height;
        const color = brey ? COLORS.amberBright : failed ? COLORS.red : COLORS.redOrange;

        // Pole.
        const pole = new Graphics();
        pole.moveTo(0, 0);
        pole.lineTo(0, height - 8);
        pole.stroke({ width: 2, color: COLORS.bayEdge, alpha: 0.8 });
        beacon.addChild(pole);

        const head = new Graphics();
        head.circle(0, 0, brey ? 7 : failed ? 6 : 5);
        head.fill({ color });
        beacon.addChild(head);
        head.name = "beacon-head";

        if (brey) {
          // Raised flag on the tallest beacon.
          const flag = new Graphics();
          flag.poly([0, -4, 14, 0, 0, 4]);
          flag.fill({ color: COLORS.amberBright, alpha: 0.9 });
          flag.y = -10;
          beacon.addChild(flag);
        }
        if (failed) {
          const sweep = new Graphics();
          sweep.moveTo(0, 0);
          sweep.lineTo(16, 0);
          sweep.stroke({ width: 1.5, color: COLORS.red, alpha: 0.6 });
          sweep.name = "beacon-sweep";
          beacon.addChild(sweep);
        }

        beacon.name = "beacon";
        container.addChild(beacon);
      }

      layout();

      // scene-mirror: test-only DOM truth mapping.
      function updateSceneMirror(): void {
        const mirror = document.getElementById("scene-mirror");
        if (!mirror) return;
        mirror.innerHTML = "";
        for (const s of state.sessions) {
          const row = document.createElement("div");
          row.dataset.stationId = s.id;
          row.dataset.state = s.state;
          row.dataset.fidelity = s.fidelity;
          row.dataset.bay = s.bay;
          row.dataset.motion = motionFor(s.state, s.fidelity, s.restored);
          row.dataset.beacon = beaconFor(s.state, s.restored);
          if (s.restored) row.dataset.restored = "true";
          row.textContent = `${s.id}|${s.state}|${s.fidelity}|${s.bay}|${row.dataset.motion}|${row.dataset.beacon}`;
          mirror.appendChild(row);
        }
      }
      updateSceneMirror();

      // Faint CRT scanline texture overlay — always present at low alpha,
      // distinct from the amber blind-pipeline overlay above it in z-order.
      const scanlines = new Graphics();
      app.stage.addChild(scanlines);
      function drawScanlines(): void {
        scanlines.clear();
        const { width, height } = app.screen;
        scanlines.setStrokeStyle({ width: 1, color: COLORS.white, alpha: 0.05 });
        for (let y = 0; y < height; y += 3) {
          scanlines.moveTo(0, y);
          scanlines.lineTo(width, y);
        }
        scanlines.stroke();
      }
      drawScanlines();

      function drawOverlay(): void {
        overlay.clear();
        const blind = !state.pipeline.verified || state.pipeline.remote_estate !== "live";
        if (!blind) return;
        const { width, height } = app.screen;
        overlay.rect(0, 0, width, height);
        overlay.fill({ color: COLORS.amber, alpha: 0.04 });
        const step = 24;
        overlay.setStrokeStyle({ width: 1, color: COLORS.amber, alpha: 0.12 });
        for (let x = -height; x < width; x += step) {
          overlay.moveTo(x, 0);
          overlay.lineTo(x + height, height);
        }
        overlay.stroke();
      }
      drawOverlay();

      function drawAmbientWash(t: number): void {
        ambientWash.clear();
        const { width, height } = app.screen;
        const hasFailed = state.sessions.some((s) => s.state === "failed");
        // Idle floor breathes slowly; busy floor is a warm steady wash.
        const breathe = ambient < 0.4 && !reducedMotion ? 0.85 + 0.15 * Math.sin(t * 0.6) : 1;
        const level = ambient * breathe;
        const color = ambient > 0.5 ? AMBIENT_LIT : AMBIENT_DIM;
        ambientWash.rect(0, 0, width, height);
        ambientWash.fill({ color, alpha: (ambient > 0.5 ? 0.05 : 0.12) * level });
        if (hasFailed) {
          // Floor-wide amber wash for FAILED stations.
          ambientWash.rect(0, 0, width, height);
          ambientWash.fill({ color: COLORS.amber, alpha: 0.05 });
        }
      }
      drawAmbientWash(0);

      app.renderer.on("resize", () => {
        layout();
        drawOverlay();
        drawScanlines();
        drawAmbientWash(performance.now() / 1000);
      });

      // 30fps cap + reduced-motion respect. Motion is time-parameterized
      // (elapsed seconds), never frame-count-parameterized.
      let elapsed = 0;
      const frameBudget = 1 / 30;
      app.ticker.maxFPS = 30;
      app.ticker.add((ticker) => {
        elapsed += ticker.deltaMS / 1000;
        if (elapsed < frameBudget) return;
        elapsed = 0;
        const t = performance.now() / 1000;
        drawAmbientWash(t);
        if (reducedMotion) return;

        for (const st of stations) {
          const s = effectiveState(st.record.state, st.record.restored);
          const motion = motionFor(st.record.state, st.record.fidelity, st.record.restored);

          if (motion === "solid" || motion === "ghost") {
            const alpha = motion === "ghost" ? 0.5 : 1;
            st.particleLayer.clear();
            if (s === "working") {
              // Rhythmic tool-head stroke.
              const stroke = Math.sin(t * 4 + st.phase);
              st.chassis.pivot.set(0, 0);
              st.chassis.rotation = motion === "solid" ? stroke * 0.02 : 0;
              if (motion === "solid") {
                spawnSparkParticles(st, t);
              } else {
                ghostOutline(st.particleLayer, t);
              }
            } else if (s === "thinking") {
              const pulse = 0.75 + 0.25 * Math.sin(t * 1.6 + st.phase);
              st.chassis.alpha = alpha * pulse;
              if (motion === "solid") {
                spawnDriftParticles(st, t);
              } else {
                ghostOutline(st.particleLayer, t);
              }
            } else if (s === "specialist") {
              const pulse = 0.85 + 0.15 * Math.sin(t * 0.8 + st.phase);
              st.chassis.alpha = alpha * pulse;
              if (motion === "solid") {
                spawnPlumeParticles(st, t);
              } else {
                ghostOutline(st.particleLayer, t);
              }
            }
          }

          if (s === "brey_required") {
            const pulse = 0.5 + 0.5 * Math.abs(Math.sin(t * 4 + st.phase));
            st.chassis.alpha = pulse;
            st.chassis.scale.set(0.95 + 0.2 * pulse);
            if (st.beacon) {
              const head = st.beacon.getChildByName?.("beacon-head") as Graphics | undefined;
              if (head) head.alpha = 0.5 + 0.5 * Math.abs(Math.sin(t * 5 + st.phase));
            }
          } else if (s === "hung") {
            st.chassis.alpha = Math.random() > 0.1 ? 1 : 0.3;
            if (st.label) {
              st.label.text = fmtElapsed(st.elapsedBase + t);
            }
          } else if (s === "fading_ended") {
            st.chassis.alpha = 0.5 + 0.3 * Math.sin(t * 1.2 + st.phase);
          } else if (s === "failed") {
            const sweep = st.container.children.find((c) => c.name === "beacon");
            if (sweep) sweep.rotation = t * 2.5;
          } else if (s === "stale_unknown") {
            // Scanline tear: a thin horizontal jitter through the chassis alpha.
            st.chassis.alpha = 0.55 + 0.1 * Math.sin(t * 8 + st.phase);
          }
        }
      });

      function spawnSparkParticles(st: StationSprite, t: number): void {
        const n = Math.min(perStationParticleBudget, 8);
        for (let i = 0; i < n; i++) {
          const seed = i * 13.37 + st.phase;
          const life = (t * 2 + seed) % 1;
          const angle = seed % (Math.PI * 2);
          const dist = life * 20;
          const x = Math.cos(angle) * dist;
          const y = Math.sin(angle) * dist * 0.5 - ST_LIFT * 0.7;
          st.particleLayer.circle(x, y, 1.4 * (1 - life));
          st.particleLayer.fill({ color: COLORS.amberBright, alpha: 0.8 * (1 - life) });
        }
      }

      function spawnDriftParticles(st: StationSprite, t: number): void {
        const n = Math.min(perStationParticleBudget, 6);
        for (let i = 0; i < n; i++) {
          const seed = i * 7.77 + st.phase;
          const life = (t * 0.4 + seed) % 1;
          const x = (Math.sin(seed * 3) * 10);
          const y = -ST_LIFT - life * 30;
          st.particleLayer.circle(x, y, 1.2 * (1 - life));
          st.particleLayer.fill({ color: COLORS.blue, alpha: 0.6 * (1 - life) });
        }
      }

      function spawnPlumeParticles(st: StationSprite, t: number): void {
        const n = Math.min(perStationParticleBudget, 5);
        for (let i = 0; i < n; i++) {
          const seed = i * 5.55 + st.phase;
          const life = (t * 0.25 + seed) % 1;
          const x = Math.sin(seed * 2) * 6 * life;
          const y = -ST_LIFT - life * 24;
          st.particleLayer.circle(x, y, 1.6 * (1 - life * 0.6));
          st.particleLayer.fill({ color: COLORS.violet, alpha: 0.5 * (1 - life) });
        }
      }

      function ghostOutline(g: Graphics, t: number): void {
        const pulse = 0.4 + 0.1 * Math.sin(t * 1.2);
        dashedDiamond(g, ST_W + 4, ST_H + 2 - ST_LIFT / 4, COLORS.textDim, pulse);
      }

      return {
        destroy: () => {
          app.destroy(true, { children: true });
        },
      };
    });
}

function fmtElapsed(secs: number): string {
  const s = Math.max(0, Math.floor(secs));
  const hh = Math.floor(s / 3600);
  const mm = Math.floor((s % 3600) / 60);
  const ss = s % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return hh > 0 ? `${hh}:${pad(mm)}:${pad(ss)}` : `${pad(mm)}:${pad(ss)}`;
}
