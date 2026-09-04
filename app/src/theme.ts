// Original retro-futurist command-center palette. No external fonts/assets —
// system UI + monospace fallback stack only.

export const FONT_STACK =
  '"Berkeley Mono", "JetBrains Mono", ui-monospace, "SF Mono", Menlo, Consolas, "Courier New", monospace';

export const COLORS = {
  bg: 0x0a0d12,
  bgHex: "#0a0d12",
  floor: 0x141922,
  floorEdge: 0x232b38,
  bayFill: 0x1b2230,
  bayFillDim: 0x151b24,
  bayEdge: 0x3a4a63,
  bayEdgeUnresolved: 0x4a3a2a,
  text: 0xd8e2f0,
  textDim: 0x6d7a8f,
  amber: 0xffb020,
  amberBright: 0xffcf5c,
  green: 0x35d488,
  blue: 0x4fa8ff,
  violet: 0xb07bff,
  red: 0xff4d4f,
  redOrange: 0xff7a3d,
  gray: 0x7a8496,
  white: 0xf2f5fa,
} as const;

// state -> primary color (used by both Pixi floor and the React marquee).
export const STATE_COLOR: Record<string, number> = {
  working: COLORS.green,
  thinking: COLORS.blue,
  specialist: COLORS.violet,
  waiting_on_agent: COLORS.amber,
  waiting_on_system: COLORS.amber,
  blocked: COLORS.amber,
  brey_required: COLORS.amberBright,
  failed: COLORS.red,
  hung: COLORS.redOrange,
  idle: COLORS.gray,
  completed: COLORS.white,
  stale_unknown: COLORS.gray,
  fading_ended: COLORS.gray,
};

// Ambient floor-wash colors — warm/lit when WORKING+THINKING is high, a cool
// dim breathing wash when the floor is mostly idle. See computeAmbient() in
// floor.ts (the #1 glance signal — brightness == health).
export const AMBIENT_LIT = 0xffb46a;
export const AMBIENT_DIM = 0x141a24;

export const STATE_LABEL: Record<string, string> = {
  working: "WORKING",
  thinking: "THINKING",
  specialist: "SPECIALIST",
  waiting_on_agent: "WAITING/AGENT",
  waiting_on_system: "WAITING/SYSTEM",
  blocked: "BLOCKED",
  brey_required: "BREY REQUIRED",
  failed: "FAILED",
  hung: "HUNG",
  idle: "IDLE",
  completed: "COMPLETED",
  stale_unknown: "STALE/UNKNOWN",
  fading_ended: "FADING/ENDED",
};
