// V-04 mode overlays — DOM panels layered over (or replacing) the floor for
// PROJECT FOCUS, INCIDENT and DEEP DEBUG. AMBIENT needs no overlay of its
// own: it dims the existing chrome via CSS (`data-mode="ambient"` on the
// app root) and leans on the floor's own reduced-motion/perf profile (see
// `FloorRenderOptions.getMode` in floor.ts).

import type { BayName, FloorState, SessionRecord } from "./state";
import { isOpusModel } from "./state";
import { STATE_TABLE } from "./states";

function fmtElapsed(secs: number): string {
  const s = Math.max(0, Math.floor(secs));
  const hh = Math.floor(s / 3600);
  const mm = Math.floor((s % 3600) / 60);
  const ss = s % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return hh > 0 ? `${hh}:${pad(mm)}:${pad(ss)}` : `${pad(mm)}:${pad(ss)}`;
}

/** PROJECT FOCUS — one bay fills the screen: enlarged stations (model /
 *  effort / elapsed / task label), the bay's routines (name + status),
 *  a git plinth, test-rack/heartbeat check states, and an enlarged output
 *  shelf with per-type counts (hatched = UNKNOWN). */
export function ProjectFocus({
  state,
  bay,
  onCycle,
}: {
  state: FloorState;
  bay: BayName;
  onCycle: (dir: -1 | 1) => void;
}) {
  const sessions = state.sessions.filter((s) => s.bay === bay);
  const routines = state.routines.filter((r) => r.bay === bay);
  const checks = state.checks.filter((c) => c.bay === bay);
  const shelf = state.output_shelf[bay] ?? [];

  const gitCheck = checks.find((c) => /git/i.test(c.label));

  return (
    <div className="mode-panel focus-panel" data-testid="focus-panel">
      <div className="focus-header">
        <button className="focus-nav" onClick={() => onCycle(-1)} aria-label="previous bay">
          ←
        </button>
        <h2 className="focus-bay-name">{bay}</h2>
        <button className="focus-nav" onClick={() => onCycle(1)} aria-label="next bay">
          →
        </button>
      </div>

      <div className="focus-stations">
        {sessions.length === 0 && <div className="focus-empty">No sessions in this bay.</div>}
        {sessions.map((s) => (
          <FocusStation key={s.id} session={s} />
        ))}
      </div>

      <div className="focus-wall">
        <section className="focus-routines">
          <h3>ROUTINES</h3>
          {routines.length === 0 && <div className="focus-empty">none</div>}
          <ul>
            {routines.map((r) => (
              <li key={r.id} data-testid={`routine-${r.id}`}>
                <span className="routine-name">{r.name}</span>
                <span className="routine-status">
                  {!r.enabled
                    ? "DISABLED"
                    : r.stale
                      ? "STALE"
                      : r.overdue
                        ? "OVERDUE"
                        : `NEXT ${r.next_run_at ?? "n/a"}`}
                </span>
              </li>
            ))}
          </ul>
        </section>

        <section className="focus-plinth" data-testid="git-plinth">
          <h3>GIT PLINTH</h3>
          <div>
            {gitCheck
              ? `${gitCheck.label}: ${gitCheck.ok === null ? "UNKNOWN" : gitCheck.ok ? "CLEAN" : "DIRTY/UNPUSHED"}`
              : "no git check for this bay"}
          </div>
        </section>

        <section className="focus-checks" data-testid="check-rack">
          <h3>TEST RACK / HEARTBEAT</h3>
          <ul>
            {checks.map((c) => (
              <li key={c.id}>
                {c.label}: {c.ok === null ? "UNKNOWN" : c.ok ? "OK" : "FAIL"}
              </li>
            ))}
          </ul>
        </section>

        <section className="focus-shelf" data-testid="focus-shelf">
          <h3>OUTPUT SHELF</h3>
          <ul>
            {shelf.map((slot, i) => (
              <li key={i} className={slot.count === null ? "shelf-unknown" : ""}>
                {slot.kind.toUpperCase()}: {slot.count === null ? "UNKNOWN" : slot.count}
              </li>
            ))}
          </ul>
        </section>
      </div>
    </div>
  );
}

function FocusStation({ session }: { session: SessionRecord }) {
  const spec = STATE_TABLE[session.state];
  return (
    <div className="focus-station" data-station-id={session.id}>
      <div className="focus-station-state">{spec.label}</div>
      <div className="focus-station-label">{session.label}</div>
      <div className="focus-station-meta">
        <span>MODEL {session.model_current ?? session.model ?? "n/a"}</span>
        <span>EFFORT {isOpusModel(session.model_current ?? session.model) ? "HIGH (OPUS)" : "STANDARD"}</span>
        <span>ELAPSED {fmtElapsed(session.elapsed_secs)}</span>
      </div>
    </div>
  );
}

/** INCIDENT — floor desaturates except the offending bay/station, which
 *  fills ~60% of the screen with a fault detail panel. Only ever entered
 *  (auto or otherwise) for an *observed* fault — see `detectIncident` in
 *  modes.ts. */
export function IncidentPanel({ station, bay }: { station: SessionRecord | null; bay: BayName | null }) {
  return (
    <div className="mode-panel incident-panel" data-testid="incident-panel">
      <div className="incident-fault">
        <h2>INCIDENT — {bay ?? "UNKNOWN BAY"}</h2>
        {station ? (
          <dl>
            <dt>STATION</dt>
            <dd>{station.id}</dd>
            <dt>STATE</dt>
            <dd>{STATE_TABLE[station.state].label}</dd>
            <dt>ELAPSED</dt>
            <dd>{fmtElapsed(station.elapsed_secs)}</dd>
            <dt>TASK</dt>
            <dd>{station.label}</dd>
            <dt>LAST OBSERVED</dt>
            <dd>just now</dd>
            <dt>FIDELITY</dt>
            <dd>{station.fidelity.toUpperCase()}</dd>
          </dl>
        ) : (
          <div>Routine overdue in this bay — no single offending station.</div>
        )}
      </div>
    </div>
  );
}

/** DEEP DEBUG — L3 station detail + L4 redacted event tape, per-observer
 *  health (capabilities + last error), and the MACHINES list. */
/** Memory discipline (V-05): the event tape never renders more than this
 *  many rows, however large the underlying fixture/feed gets — a live feed
 *  is an append-only stream and DOM nodes are real memory, so this is a
 *  hard ring-buffer cap, not just a display truncation. Keeps the newest
 *  events (the tail), which is what DEEP DEBUG cares about. */
const TAPE_RING_CAP = 200;

export function DeepDebug({ state, bay }: { state: FloorState; bay: BayName | null }) {
  const sessions = bay ? state.sessions.filter((s) => s.bay === bay) : state.sessions;
  const fullTape = state.tape ?? [];
  const tape = fullTape.length > TAPE_RING_CAP ? fullTape.slice(-TAPE_RING_CAP) : fullTape;
  const machines = state.machines ?? [];

  return (
    <div className="mode-panel debug-panel" data-testid="debug-panel">
      <div className="debug-col debug-l3">
        <h3>L3 STATION DETAIL</h3>
        <ul>
          {sessions.map((s) => (
            <li key={s.id}>
              {s.id} — {s.state} [{s.fidelity}] {s.label}
            </li>
          ))}
        </ul>
      </div>

      <div className="debug-col debug-l4">
        <h3>L4 EVENT TAPE</h3>
        <div className="tape" data-testid="tape">
          {tape.map((ev, i) => (
            <div className="tape-row" data-testid="tape-row" key={i}>
              <span>{ev.ts}</span>
              <span>{ev.source}</span>
              <span>{ev.kind}</span>
              <span>{ev.entity}</span>
              <span>{ev.state}</span>
              <span>{ev.fidelity}</span>
            </div>
          ))}
        </div>
      </div>

      <div className="debug-col debug-side">
        <section data-testid="observer-health">
          <h3>OBSERVER HEALTH</h3>
          <ul>
            {state.observers.map((o) => (
              <li key={o.name}>
                {o.name}: {o.status.toUpperCase()} — caps: {(o.capabilities ?? []).join(", ") || "none"}
                {o.last_error ? ` — ${o.last_error}` : ""}
              </li>
            ))}
          </ul>
        </section>
        <section data-testid="machines-list">
          <h3>MACHINES</h3>
          <ul>
            {machines.map((m) => (
              <li key={m.id}>
                {m.name}: {m.reachable ? "REACHABLE" : "UNREACHABLE"}
              </li>
            ))}
          </ul>
        </section>
      </div>
    </div>
  );
}
