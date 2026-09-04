// V-04 display modes — the five hotkey-selectable ways of looking at the
// floor. Pure logic (mode detection / bay selection) lives here so it's
// testable and reusable independent of App.tsx's React wiring.

import type { BayName, FloorState, SessionRecord } from "./state";

export type Mode = "command" | "focus" | "ambient" | "incident" | "debug";

export const MODE_LABEL: Record<Mode, string> = {
  command: "COMMAND CENTER",
  focus: "PROJECT FOCUS",
  ambient: "AMBIENT",
  incident: "INCIDENT",
  debug: "DEEP DEBUG",
};

export const HOTKEY_MODE: Record<string, Mode> = {
  "1": "command",
  "2": "focus",
  "3": "ambient",
  "4": "incident",
  "5": "debug",
};

export const MODE_STORAGE_KEY = "foundry-mode";

/** Default overdue threshold for auto-INCIDENT, in minutes (§ INCIDENT).
 *  Configurable — see `detectIncident`'s `overdueMinutes` param. */
export const DEFAULT_OVERDUE_MINUTES = 15;

export interface IncidentInfo {
  active: boolean;
  bay: BayName | null;
  station: SessionRecord | null;
  /** True when the incident is only reachable via a routine gone overdue
   *  (no FAILED/BREY_REQUIRED station) — still an incident, just without a
   *  single offending station to highlight. */
  routineOnly: boolean;
}

/** INCIDENT auto-enters only on *observed* FAILED/BREY_REQUIRED stations, or
 *  a routine overdue past the threshold — never on inferred-only signals
 *  (those get a subtle amber wash instead, see `hasInferredFault`). */
export function detectIncident(
  state: FloorState,
  overdueMinutes: number = DEFAULT_OVERDUE_MINUTES
): IncidentInfo {
  const offender = state.sessions.find(
    (s) => s.fidelity === "observed" && !s.restored && (s.state === "failed" || s.state === "brey_required")
  );
  if (offender) {
    return { active: true, bay: offender.bay, station: offender, routineOnly: false };
  }
  const overdueRoutine = state.routines.find((r) => r.enabled && r.overdue && !r.stale && !r.restored);
  if (overdueRoutine) {
    return { active: true, bay: overdueRoutine.bay, station: null, routineOnly: true };
  }
  return { active: false, bay: null, station: null, routineOnly: false };
}

/** Inferred-only FAILED/BREY_REQUIRED never snaps to INCIDENT — it gets a
 *  subtle amber wash instead (see App.tsx / style.css `.amber-wash`). */
export function hasInferredFault(state: FloorState): boolean {
  return state.sessions.some(
    (s) => s.fidelity === "inferred" && (s.state === "failed" || s.state === "brey_required")
  );
}

/** The bay with the most non-idle/non-completed sessions — used both for
 *  PROJECT FOCUS's default selection and the COMMAND CENTER autopilot
 *  drift target. */
export function mostActiveBay(state: FloorState): BayName | null {
  const counts = new Map<BayName, number>();
  for (const s of state.sessions) {
    if (s.state === "idle" || s.state === "completed" || s.state === "fading_ended") continue;
    counts.set(s.bay, (counts.get(s.bay) ?? 0) + 1);
  }
  let best: BayName | null = null;
  let bestCount = -1;
  for (const [bay, count] of counts) {
    if (count > bestCount) {
      best = bay;
      bestCount = count;
    }
  }
  return best;
}
