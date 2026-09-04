// V-03 state-mapping table — the single source of truth for how every
// StationState renders. `floor.ts` MUST read color/lightMode/motion/glyph/
// beacon/label from here rather than switching on `state` inline anywhere
// else, so the mapping stays auditable in one place.

import type { StationState } from "./state";
import { COLORS } from "./theme";

export type LightMode = "steady" | "pulse" | "flicker" | "off";
export type MotionKind = "strokes" | "breathe" | "none" | "frozen";
export type BeaconKind = "none" | "amber" | "red";
export type Glyph =
  | "circle"
  | "circle-outline"
  | "circle-faint"
  | "diamond"
  | "square"
  | "square-x"
  | "square-dim"
  | "flag"
  | "triangle"
  | "triangle-outline"
  | "hatch";

export interface StateSpec {
  color: number;
  lightMode: LightMode;
  motion: MotionKind;
  glyph: Glyph;
  beacon: BeaconKind;
  label: string;
}

/** The 12 §6 states + the fading/ended tail state. Ordering matches the
 *  mission brief. Every field here is the single source of truth for
 *  rendering — see floor.ts's use of `STATE_TABLE[record.state]`. */
export const STATE_TABLE: Record<StationState, StateSpec> = {
  working: {
    color: COLORS.green,
    lightMode: "steady",
    motion: "strokes",
    glyph: "circle",
    beacon: "none",
    label: "WORKING",
  },
  thinking: {
    color: COLORS.blue,
    lightMode: "pulse",
    motion: "breathe",
    glyph: "circle",
    beacon: "none",
    label: "THINKING",
  },
  specialist: {
    color: COLORS.violet,
    lightMode: "pulse",
    motion: "breathe",
    glyph: "diamond",
    beacon: "none",
    label: "SPECIALIST",
  },
  waiting_on_agent: {
    color: COLORS.amber,
    lightMode: "steady",
    motion: "none",
    glyph: "square",
    beacon: "none",
    label: "WAITING/AGENT",
  },
  waiting_on_system: {
    color: COLORS.amber,
    lightMode: "steady",
    motion: "none",
    glyph: "square",
    beacon: "none",
    label: "WAITING/SYSTEM",
  },
  blocked: {
    color: COLORS.amber,
    lightMode: "steady",
    motion: "none",
    glyph: "square-x",
    beacon: "none",
    label: "BLOCKED",
  },
  brey_required: {
    color: COLORS.amberBright,
    lightMode: "pulse",
    motion: "breathe",
    glyph: "flag",
    beacon: "amber",
    label: "BREY REQUIRED",
  },
  failed: {
    color: COLORS.red,
    lightMode: "steady",
    motion: "none",
    glyph: "triangle",
    beacon: "red",
    label: "FAILED",
  },
  hung: {
    color: COLORS.redOrange,
    lightMode: "flicker",
    motion: "frozen",
    glyph: "triangle-outline",
    beacon: "red",
    label: "HUNG",
  },
  idle: {
    color: COLORS.gray,
    lightMode: "off",
    motion: "none",
    glyph: "square-dim",
    beacon: "none",
    label: "IDLE",
  },
  completed: {
    color: COLORS.white,
    lightMode: "steady",
    motion: "none",
    glyph: "circle-outline",
    beacon: "none",
    label: "COMPLETED",
  },
  stale_unknown: {
    color: COLORS.gray,
    lightMode: "off",
    motion: "none",
    glyph: "hatch",
    beacon: "none",
    label: "STALE/UNKNOWN",
  },
  fading_ended: {
    color: COLORS.gray,
    lightMode: "off",
    motion: "none",
    glyph: "circle-faint",
    beacon: "none",
    label: "FADING/ENDED",
  },
};

/** Motion class exposed on the scene mirror, gated by fidelity per V-02/V-03
 *  fidelity rules: `unknown` never animates; `inferred` never renders solid
 *  motion, only a ghosted 55%-alpha version; `restored` always forces the
 *  STALE treatment (frozen/off) regardless of nominal state. */
export function motionFor(
  state: StationState,
  fidelity: "observed" | "inferred" | "unknown",
  restored?: boolean
): "solid" | "ghost" | "none" {
  if (restored) return fidelity === "observed" ? "none" : "none";
  if (fidelity === "unknown") return "none";
  const spec = STATE_TABLE[state];
  if (spec.motion === "none" || spec.motion === "frozen") return "none";
  return fidelity === "observed" ? "solid" : "ghost";
}

/** Beacon kind for a station, respecting the restored-forces-stale rule. */
export function beaconFor(state: StationState, restored?: boolean): BeaconKind {
  if (restored) return "none";
  return STATE_TABLE[state].beacon;
}

/** Effective render state: `restored` always forces STALE treatment,
 *  never trusting a restored WORKING (or any other restored state). */
export function effectiveState(state: StationState, restored?: boolean): StationState {
  return restored ? "stale_unknown" : state;
}
