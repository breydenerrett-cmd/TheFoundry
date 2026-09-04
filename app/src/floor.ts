// Pixi floor renderer — isometric 2:1 fake-3D dark industrial floor.
// Pure rendering module: takes a FloorState snapshot and draws it. No network,
// no state mutation beyond Pixi's own scene graph. See feed.ts for data.

import { Application, Container, Graphics, Text, TextStyle } from "pixi.js";
import type { BayName, FloorState, SessionRecord, StationState } from "./state";
import { BAYS } from "./state";
import { COLORS, FONT_STACK, STATE_LABEL } from "./theme";

const TILE_W = 220;
const TILE_H = 110;
// Grid spacing is wider than the platform diamond itself so adjacent bays'
// signage, stations and output shelves have breathing room.
const GRID_W = TILE_W * 1.7;
const GRID_H = TILE_H * 2.6;

// Simple 4x2 grid layout for the 7 bays (6 real + UNRESOLVED), projected
// isometrically so the floor reads as a raised industrial deck.
const BAY_GRID: Record<BayName, { col: number; row: number }> = {
  "SPORTS LAB": { col: 0, row: 0 },
  "AI BUSINESS COMPLEX": { col: 1, row: 0 },
  SERVERFORGE: { col: 2, row: 0 },
  "MUSIC LAB": { col: 3, row: 0 },
  EXPERIMENTS: { col: 0, row: 1 },
  "PERSONAL/MISC": { col: 1, row: 1 },
  UNRESOLVED: { col: 2, row: 1 },
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

interface StationSprite {
  container: Container;
  body: Graphics;
  fidelityOverlay: Graphics;
  record: SessionRecord;
  phase: number;
}

export interface FloorHandle {
  destroy: () => void;
}

export function mountFloor(canvasHost: HTMLDivElement, state: FloorState): Promise<FloorHandle> {
  const app = new Application();
  const reducedMotion = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;

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

      const overlay = new Graphics();
      app.stage.addChild(overlay);

      const stations: StationSprite[] = [];

      function layout(): void {
        world.removeChildren();
        const { width } = app.screen;
        world.x = width / 2;
        world.y = 70;
        const scale = Math.min(1, width / 1700);
        world.scale.set(Math.max(0.55, scale));

        const shelfMap = state.output_shelf;

        for (const bay of BAYS) {
          const grid = BAY_GRID[bay];
          const p = isoProject(grid.col, grid.row);
          const bayContainer = new Container();
          bayContainer.x = p.x;
          bayContainer.y = p.y;
          world.addChild(bayContainer);

          drawBayPlatform(bayContainer, bay);

          const sessions = state.sessions.filter((s) => s.bay === bay);
          const cols = 3;
          sessions.forEach((s, i) => {
            const sx = (i % cols) * 56 - 56;
            const sy = Math.floor(i / cols) * 34 - 10;
            const sprite = buildStation(s);
            sprite.container.x = sx;
            sprite.container.y = sy;
            bayContainer.addChild(sprite.container);
            stations.push(sprite);
          });

          drawOutputShelf(bayContainer, shelfMap[bay] ?? []);

          const hasFault = sessions.some((s) => s.state === "failed" || s.state === "hung");
          const hasBreyRequired = sessions.some((s) => s.state === "brey_required");
          if (hasFault || hasBreyRequired) {
            drawFaultBeacon(bayContainer, hasBreyRequired);
          }
        }
      }

      function drawBayPlatform(container: Container, bay: BayName): void {
        const isUnresolved = bay === "UNRESOLVED";
        const g = new Graphics();
        const w = TILE_W;
        const h = TILE_H;
        const fill = isUnresolved ? COLORS.bayFillDim : COLORS.bayFill;
        const edge = isUnresolved ? COLORS.bayEdgeUnresolved : COLORS.bayEdge;
        g.poly([0, -h / 2, w / 2, 0, 0, h / 2, -w / 2, 0]);
        g.fill({ color: fill, alpha: isUnresolved ? 0.55 : 0.85 });
        g.stroke({ width: 2, color: edge, alpha: 0.9 });
        container.addChild(g);

        if (isUnresolved) {
          const hatch = new Graphics();
          hatch.poly([0, -h / 2, w / 2, 0, 0, h / 2, -w / 2, 0]);
          hatch.fill({ color: 0, alpha: 0 });
          hatchPattern(hatch, w, h / 2, COLORS.bayEdgeUnresolved, 0.25);
          hatch.y = -h / 4;
          container.addChild(hatch);
        }

        const label = new Text({
          text: bay + (isUnresolved ? "  (not guessed)" : ""),
          style: new TextStyle({
            fontFamily: FONT_STACK,
            fontSize: 11,
            fill: isUnresolved ? COLORS.textDim : COLORS.text,
            letterSpacing: 1,
          }),
        });
        label.anchor.set(0.5, 1);
        label.y = -h / 2 - 6;
        container.addChild(label);
      }

      function buildStation(record: SessionRecord): StationSprite {
        const container = new Container();
        const body = new Graphics();
        container.addChild(body);
        const fidelityOverlay = new Graphics();
        container.addChild(fidelityOverlay);
        drawStationShape(body, record.state);
        drawFidelityOverlay(fidelityOverlay, record);
        return { container, body, fidelityOverlay, record, phase: Math.random() * Math.PI * 2 };
      }

      function drawStationShape(g: Graphics, state: StationState): void {
        g.clear();
        const color = STATE_COLOR_LOCAL(state);
        switch (state) {
          case "working":
          case "thinking":
            g.circle(0, 0, 8);
            g.fill({ color });
            break;
          case "specialist":
            g.poly([0, -9, 9, 0, 0, 9, -9, 0]);
            g.fill({ color });
            g.moveTo(0, -9);
            g.lineTo(0, -18);
            g.stroke({ width: 2, color, alpha: 0.6 });
            break;
          case "waiting_on_agent":
            g.rect(-7, -7, 14, 14);
            g.stroke({ width: 2, color });
            g.moveTo(0, 7);
            g.lineTo(0, 16);
            g.stroke({ width: 1, color, alpha: 0.6 });
            break;
          case "waiting_on_system":
            g.rect(-7, -7, 14, 14);
            g.fill({ color, alpha: 0.35 });
            g.stroke({ width: 2, color });
            g.moveTo(0, 7);
            g.lineTo(0, 16);
            g.stroke({ width: 1, color, alpha: 0.6 });
            break;
          case "blocked":
            g.rect(-7, -7, 14, 14);
            g.stroke({ width: 2, color });
            g.moveTo(-5, -5);
            g.lineTo(5, 5);
            g.moveTo(5, -5);
            g.lineTo(-5, 5);
            g.stroke({ width: 2, color });
            break;
          case "brey_required":
            g.circle(0, 0, 9);
            g.fill({ color });
            g.moveTo(0, -9);
            g.lineTo(0, -20);
            g.stroke({ width: 2, color });
            g.poly([0, -20, 8, -16, 0, -12]);
            g.fill({ color });
            break;
          case "failed":
            g.poly([0, -9, 9, 8, -9, 8]);
            g.fill({ color });
            break;
          case "hung":
            g.poly([0, -9, 9, 8, -9, 8]);
            g.fill({ color, alpha: 0.8 });
            g.stroke({ width: 2, color: COLORS.white, alpha: 0.6 });
            break;
          case "idle":
            g.rect(-5, -5, 10, 10);
            g.fill({ color, alpha: 0.6 });
            break;
          case "completed":
            g.circle(0, 0, 7);
            g.stroke({ width: 2, color: COLORS.white, alpha: 0.9 });
            break;
          case "stale_unknown":
            g.rect(-7, -7, 14, 14);
            g.fill({ color, alpha: 0.25 });
            hatchPattern(g, 14, 14, color, 0.6);
            g.x -= 0; // no-op, keeps shape anchored
            break;
          case "fading_ended":
            g.circle(0, 0, 8);
            g.fill({ color, alpha: 0.25 });
            g.stroke({ width: 1, color, alpha: 0.4 });
            break;
        }
      }

      function drawFidelityOverlay(g: Graphics, record: SessionRecord): void {
        g.clear();
        if (record.fidelity === "inferred") {
          dashedCircle(g, 13, COLORS.textDim, 0.8);
        } else if (record.fidelity === "unknown") {
          hatchPattern(g, 20, 20, COLORS.textDim, 0.4);
          g.x = -10;
          g.y = -10;
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

      function drawFaultBeacon(container: Container, breyRequired: boolean): void {
        const beacon = new Graphics();
        beacon.y = -TILE_H / 2 - 30;
        const color = breyRequired ? COLORS.amberBright : COLORS.red;
        beacon.circle(0, 0, 5);
        beacon.fill({ color });
        container.addChild(beacon);
        (beacon as unknown as { __beacon: boolean }).__beacon = true;
        beacon.name = "beacon";
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
          row.textContent = `${s.id}|${s.state}|${s.fidelity}|${s.bay}`;
          mirror.appendChild(row);
        }
      }
      updateSceneMirror();

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

      app.renderer.on("resize", () => {
        layout();
        drawOverlay();
      });

      // 30fps cap + reduced-motion respect.
      let elapsed = 0;
      const frameBudget = 1 / 30;
      app.ticker.maxFPS = 30;
      app.ticker.add((ticker) => {
        if (reducedMotion) return;
        elapsed += ticker.deltaMS / 1000;
        if (elapsed < frameBudget) return;
        elapsed = 0;
        const t = performance.now() / 1000;
        for (const st of stations) {
          const s = st.record.state;
          if (s === "thinking") {
            const pulse = 0.8 + 0.2 * Math.sin(t * 2 + st.phase);
            st.body.alpha = pulse;
          } else if (s === "brey_required") {
            const pulse = 0.55 + 0.45 * Math.abs(Math.sin(t * 3.2 + st.phase));
            st.body.alpha = pulse;
            st.body.scale.set(0.95 + 0.15 * Math.abs(Math.sin(t * 3.2 + st.phase)));
          } else if (s === "hung") {
            st.body.alpha = Math.random() > 0.08 ? 1 : 0.35;
          } else if (s === "fading_ended") {
            st.body.alpha = 0.5 + 0.3 * Math.sin(t * 1.2 + st.phase);
          } else if (s === "failed") {
            st.body.rotation = t * 1.5;
          }
        }
      });

      return {
        destroy: () => {
          app.destroy(true, { children: true });
        },
      };
    });
}

// Re-exported locally to avoid a circular import loop with theme.ts's
// STATE_COLOR table (kept here for clarity at each draw call site).
import { STATE_COLOR } from "./theme";
function STATE_COLOR_LOCAL(state: StationState): number {
  return STATE_COLOR[state] ?? COLORS.gray;
}
