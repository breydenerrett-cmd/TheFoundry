import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { startFeed, type FeedStatus } from "./feed";
import { mountFloor, type FloorHandle } from "./floor";
import { Marquee } from "./Marquee";
import type { BayName, FloorState, SessionRecord } from "./state";
import { BAYS } from "./state";
import {
  detectIncident,
  hasInferredFault,
  HOTKEY_MODE,
  mostActiveBay,
  MODE_STORAGE_KEY,
  type Mode,
} from "./modes";
import { DeepDebug, IncidentPanel, ProjectFocus } from "./ModeOverlays";

const AUTOPILOT_IDLE_MS = 60_000;
const AUTOPILOT_DRIFT_MS = 20_000;

/** Test-only overrides for the autopilot timers, via `?autopilotIdleMs=` /
 *  `?autopilotDwellMs=` query params — lets tests exercise the COMMAND
 *  CENTER -> PROJECT FOCUS -> back drift without waiting on real 60s/20s
 *  timers. Falls back to the production defaults when absent or invalid. */
function readAutopilotTimings(): { idleMs: number; driftMs: number } {
  try {
    const params = new URLSearchParams(window.location.search);
    const idle = Number(params.get("autopilotIdleMs"));
    const dwell = Number(params.get("autopilotDwellMs"));
    return {
      idleMs: Number.isFinite(idle) && idle > 0 ? idle : AUTOPILOT_IDLE_MS,
      driftMs: Number.isFinite(dwell) && dwell > 0 ? dwell : AUTOPILOT_DRIFT_MS,
    };
  } catch {
    return { idleMs: AUTOPILOT_IDLE_MS, driftMs: AUTOPILOT_DRIFT_MS };
  }
}

function loadStoredMode(): Mode | null {
  try {
    const v = localStorage.getItem(MODE_STORAGE_KEY);
    if (v === "command" || v === "focus" || v === "ambient" || v === "incident" || v === "debug") {
      return v;
    }
  } catch {
    // localStorage unavailable (private mode, etc.) — fall through.
  }
  return null;
}

function storeMode(mode: Mode): void {
  try {
    localStorage.setItem(MODE_STORAGE_KEY, mode);
  } catch {
    // ignore — per-viewer convenience only.
  }
}

/** L-01: a placeholder floor with nothing in it — what renders (under the
 *  WATCHER DOWN overlay) when the http feed has never successfully returned
 *  any state at all. Never a fixture, never fabricated occupancy. */
const EMPTY_FLOOR_STATE: FloorState = {
  generated_at: new Date(0).toISOString(),
  sessions: [],
  routines: [],
  checks: [],
  observers: [],
  pipeline: {
    verified: false,
    remote_estate: "not_running",
    last_sync_age_secs: null,
    last_output_age_secs: null,
    next_routine: null,
  },
  output_shelf: Object.fromEntries(BAYS.map((b) => [b, []])) as unknown as FloorState["output_shelf"],
  machines: [],
  tape: [],
};

/** L-01 truth rule: whenever the feed is `stale` or `down`, every station
 *  renders through the existing "never trust a restored record" pipeline
 *  (`effectiveState()` in states.ts) regardless of its nominal state — a
 *  frozen/unreachable watcher must never show a station as confidently
 *  WORKING. Returns the same object when the feed is live/fixture. */
function withFeedStaleness(state: FloorState, liveness: FeedStatus["liveness"] | null): FloorState {
  if (liveness !== "stale" && liveness !== "down") return state;
  const sessions: SessionRecord[] = state.sessions.map((s) => ({ ...s, restored: true }));
  return { ...state, sessions };
}

export function App() {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const [rawState, setRawState] = useState<FloorState | null>(null);
  const [feedStatus, setFeedStatus] = useState<FeedStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  // The renderer always gets SOME FloorState once we know the feed kind —
  // real data when we have it, or an honestly-empty placeholder (never a
  // fixture) while the http feed has never returned anything at all — so
  // the WATCHER DOWN overlay always has a floor underneath it to darken.
  const state = useMemo(() => {
    const base = rawState ?? (feedStatus ? EMPTY_FLOOR_STATE : null);
    if (!base) return null;
    return withFeedStaleness(base, feedStatus?.liveness ?? null);
  }, [rawState, feedStatus]);

  const [mode, setModeRaw] = useState<Mode>(() => loadStoredMode() ?? "command");
  const [focusBay, setFocusBay] = useState<BayName | null>(null);
  const modeRef = useRef(mode);
  modeRef.current = mode;

  const preIncidentMode = useRef<Mode>("command");
  const dismissedIncidentKey = useRef<string | null>(null);
  const autopilotArmedAt = useRef<number>(Date.now());
  const autopilotDriftTimer = useRef<number | null>(null);
  const autopilotReturnMode = useRef<Mode>("command");

  const setMode = useCallback((next: Mode) => {
    setModeRaw((prev) => {
      if (next !== "incident" && prev === "incident") {
        // manual exit from an auto-entered incident — don't snap right back
        // on the same fault.
      }
      return next;
    });
    storeMode(next);
    autopilotArmedAt.current = Date.now();
  }, []);

  useEffect(() => {
    try {
      const handle = startFeed((s, status) => {
        if (s) setRawState(s);
        setFeedStatus(status);
      });
      return () => handle.stop();
    } catch (e) {
      setError(String(e));
      return undefined;
    }
  }, []);

  // Default the focused bay to the most active one once state loads.
  useEffect(() => {
    if (state && focusBay === null) {
      setFocusBay(mostActiveBay(state) ?? BAYS[0]);
    }
  }, [state, focusBay]);

  // INCIDENT auto-entry: observed-only FAILED/BREY_REQUIRED, or a routine
  // overdue past the threshold. Never on inferred-only signals (those get
  // the amber wash below instead of a takeover). Re-arms on a *new*
  // incident even after a manual Esc dismissal of a previous one.
  useEffect(() => {
    if (!state) return;
    const incident = detectIncident(state);
    const key = incident.active ? `${incident.bay}:${incident.station?.id ?? "routine"}` : null;
    if (incident.active && key !== dismissedIncidentKey.current && modeRef.current !== "incident") {
      preIncidentMode.current = modeRef.current;
      setMode("incident");
    }
    if (!incident.active && modeRef.current === "incident") {
      setMode(preIncidentMode.current);
    }
  }, [state, setMode]);

  const cycleFocusBay = useCallback(
    (dir: -1 | 1) => {
      const real: BayName[] = BAYS.filter((b) => b !== "UNRESOLVED");
      setFocusBay((cur) => {
        const idx = Math.max(0, real.indexOf(cur ?? real[0]));
        const next = real[(idx + dir + real.length) % real.length];
        return next;
      });
    },
    []
  );

  // Hotkeys 1-5, Esc, and (in PROJECT FOCUS) arrow keys to cycle bays.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      autopilotArmedAt.current = Date.now();
      if (e.key === "Escape") {
        if (modeRef.current === "incident") {
          dismissedIncidentKey.current = state ? incidentKey(state) : null;
        }
        setMode("command");
        return;
      }
      const target = HOTKEY_MODE[e.key];
      if (target) {
        setMode(target);
        return;
      }
      if (modeRef.current === "focus" && (e.key === "ArrowLeft" || e.key === "ArrowRight")) {
        cycleFocusBay(e.key === "ArrowLeft" ? -1 : 1);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [cycleFocusBay, setMode, state]);

  // Autopilot: after 60s idle in COMMAND CENTER, drift into PROJECT FOCUS
  // on the most active bay for 20s, then return. Any observed
  // FAILED/BREY_REQUIRED still snaps to INCIDENT regardless (handled by the
  // detectIncident effect above, which runs independent of this timer).
  useEffect(() => {
    if (!state) return;
    const { idleMs, driftMs } = readAutopilotTimings();
    const pollMs = Math.min(1000, Math.max(50, Math.floor(idleMs / 4)));
    const iv = window.setInterval(() => {
      if (modeRef.current !== "command") return;
      const idleFor = Date.now() - autopilotArmedAt.current;
      if (idleFor >= idleMs && autopilotDriftTimer.current === null) {
        autopilotReturnMode.current = "command";
        setFocusBay(mostActiveBay(state) ?? BAYS[0]);
        setModeRaw("focus");
        autopilotDriftTimer.current = window.setTimeout(() => {
          autopilotDriftTimer.current = null;
          setModeRaw((cur) => (cur === "focus" ? "command" : cur));
          autopilotArmedAt.current = Date.now();
        }, driftMs);
      }
    }, pollMs);
    return () => window.clearInterval(iv);
  }, [state]);

  useEffect(() => {
    if (!state || !hostRef.current) return;
    let handle: FloorHandle | null = null;
    let disposed = false;
    mountFloor(hostRef.current, state, {
      getMode: () => modeRef.current,
      onBayClick: (bay) => {
        autopilotArmedAt.current = Date.now();
        setFocusBay(bay);
        setMode("focus");
      },
    }).then((h) => {
      if (disposed) {
        h.destroy();
      } else {
        handle = h;
      }
    });
    return () => {
      disposed = true;
      handle?.destroy();
    };
  }, [state]);

  const incident = useMemo(() => (state ? detectIncident(state) : null), [state]);
  const inferredFault = useMemo(() => (state ? hasInferredFault(state) : false), [state]);

  const liveness: FeedStatus["liveness"] = feedStatus?.liveness ?? "down";
  const feedDown = liveness === "down";
  const feedStale = liveness === "stale";
  const now = Date.now();
  const okSecsAgo =
    feedStatus?.lastFetchOkAt != null ? Math.max(0, Math.round((now - feedStatus.lastFetchOkAt) / 1000)) : null;
  const frozenSecs =
    feedStatus?.lastChangedAt != null ? Math.max(0, Math.round((now - feedStatus.lastChangedAt) / 1000)) : null;

  return (
    <div className={`app-shell mode-${mode}`} data-mode={mode} data-feed={liveness}>
      {state && <Marquee state={state} mode={mode} feed={feedStatus} />}
      <div className="floor-host" ref={hostRef} />
      {state && inferredFault && mode !== "incident" && <div className="amber-wash" data-testid="inferred-fault-wash" />}
      {(feedDown || feedStale) && (
        <div className="feed-overlay" data-testid="feed-overlay">
          <div className="feed-overlay-label">
            {feedDown ? "WATCHER DOWN" : "STALE FEED"}
          </div>
          <div className="feed-overlay-detail">
            {feedDown
              ? `FEED: DOWN (last ok ${okSecsAgo ?? "n/a"}s ago)`
              : `FEED: STALE (seq frozen ${frozenSecs ?? "n/a"}s)`}
          </div>
        </div>
      )}
      {error && <div className="load-error">FLOOR LOAD ERROR: {error}</div>}

      {state && mode === "focus" && focusBay && (
        <ProjectFocus state={state} bay={focusBay} onCycle={cycleFocusBay} />
      )}
      {state && mode === "incident" && incident && (
        <IncidentPanel station={incident.station} bay={incident.bay} />
      )}
      {state && mode === "debug" && <DeepDebug state={state} bay={focusBay} />}

      {/* Test-only truth mirror — see floor.ts updateSceneMirror(). */}
      <div id="scene-mirror" hidden />
    </div>
  );
}

function incidentKey(state: FloorState): string | null {
  const incident = detectIncident(state);
  return incident.active ? `${incident.bay}:${incident.station?.id ?? "routine"}` : null;
}
