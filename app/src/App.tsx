import { useEffect, useRef, useState } from "react";
import { loadFloorState } from "./feed";
import { mountFloor, type FloorHandle } from "./floor";
import { Marquee } from "./Marquee";
import type { FloorState } from "./state";

export function App() {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const [state, setState] = useState<FloorState | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    loadFloorState()
      .then((s) => {
        if (!cancelled) setState(s);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!state || !hostRef.current) return;
    let handle: FloorHandle | null = null;
    let disposed = false;
    mountFloor(hostRef.current, state).then((h) => {
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

  return (
    <div className="app-shell">
      {state && <Marquee state={state} />}
      <div className="floor-host" ref={hostRef} />
      {error && <div className="load-error">FLOOR LOAD ERROR: {error}</div>}
      {/* Test-only truth mirror — see floor.ts updateSceneMirror(). */}
      <div id="scene-mirror" hidden />
    </div>
  );
}
