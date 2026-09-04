// V-05 performance ladder + measurement instrumentation.
//
// The ladder maps each display mode to a target frame rate, further capped
// to a near-idle 2fps whenever the page is hidden (`document.hidden`) or the
// floor canvas itself is scrolled offscreen (IntersectionObserver). All
// animation in `floor.ts` is time-parameterized (driven by `performance.now()`
// seconds, never by frame count), so changing the ladder step only changes
// how often we sample that timeline — never its speed.

export type PerfMode = "command" | "focus" | "ambient" | "incident" | "debug";

/** fps target per mode, before the hidden/offscreen override. */
export const MODE_FPS: Record<PerfMode, number> = {
  debug: 60,
  command: 30,
  focus: 30,
  incident: 30,
  ambient: 12,
};

/** fps used whenever the page is hidden or the canvas is offscreen,
 *  regardless of mode — there is nothing worth animating for nobody. */
export const HIDDEN_FPS = 2;

export interface PerfLadderOptions {
  getMode: () => PerfMode;
  /** Element whose visibility gates the ladder (the canvas host). Optional —
   *  omitted in environments without IntersectionObserver (falls back to
   *  document.hidden only). */
  el?: Element | null;
}

export interface PerfLadder {
  /** Current target fps, recomputed lazily on each call. */
  targetFps: () => number;
  destroy: () => void;
}

/** Builds a live fps-target function reflecting mode + page visibility +
 *  canvas intersection. Call `targetFps()` from the ticker each frame (or
 *  whenever `app.ticker.maxFPS` needs re-checking); it's cheap (no work
 *  beyond reading cached booleans updated by event listeners). */
export function createPerfLadder(opts: PerfLadderOptions): PerfLadder {
  let offscreen = false;
  let observer: IntersectionObserver | null = null;

  if (opts.el && typeof IntersectionObserver !== "undefined") {
    observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          offscreen = !entry.isIntersecting;
        }
      },
      { threshold: 0 }
    );
    observer.observe(opts.el);
  }

  function targetFps(): number {
    const hidden = typeof document !== "undefined" && document.hidden;
    if (hidden || offscreen) return HIDDEN_FPS;
    return MODE_FPS[opts.getMode()] ?? 30;
  }

  return {
    targetFps,
    destroy: () => {
      observer?.disconnect();
    },
  };
}

/** Measurement surface exposed as `window.__foundryPerf` for the Playwright
 *  perf harness (`tests/perf.spec.ts`). Ring-buffered so long-running pages
 *  never grow this unbounded. */
export interface FoundryPerfStats {
  /** Frames rendered (ticker callback invocations) since `reset()`. */
  frames: number;
  /** Wall-clock ms since `reset()`. */
  wallMs: number;
  /** Sum of JS busy time (ms) spent inside the ticker callback since reset. */
  busyMs: number;
  /** Achieved fps over the measured window. */
  fps: number;
  /** Main-thread busy % of wall time over the measured window. */
  busyPct: number;
  /** Last known particle count drawn in one frame (proxy for draw calls when
   *  renderer.gl/stats isn't exposed). */
  particles: number;
  reset: () => void;
}

const RING_SIZE = 240; // ~20s at 12fps floor .. plenty of headroom either way

export function installPerfStats(): FoundryPerfStats {
  let frameTimes: number[] = [];
  let busyTimes: number[] = [];
  let startedAt = performance.now();
  let particles = 0;

  const stats: FoundryPerfStats = {
    get frames() {
      return frameTimes.length;
    },
    get wallMs() {
      return performance.now() - startedAt;
    },
    get busyMs() {
      return busyTimes.reduce((a, b) => a + b, 0);
    },
    get fps() {
      const wall = performance.now() - startedAt;
      return wall > 0 ? (frameTimes.length / wall) * 1000 : 0;
    },
    get busyPct() {
      const wall = performance.now() - startedAt;
      const busy = busyTimes.reduce((a, b) => a + b, 0);
      return wall > 0 ? (busy / wall) * 100 : 0;
    },
    get particles() {
      return particles;
    },
    reset: () => {
      frameTimes = [];
      busyTimes = [];
      startedAt = performance.now();
    },
  } as FoundryPerfStats;

  (window as unknown as { __foundryPerf: FoundryPerfStats }).__foundryPerf = stats;

  return Object.assign(stats, {
    // internal hooks used by floor.ts — not part of the public shape but
    // attached to the same object so floor.ts can import & call them.
    _recordFrame(busyMsThisFrame: number, particleCount: number) {
      frameTimes.push(performance.now());
      if (frameTimes.length > RING_SIZE) frameTimes.shift();
      busyTimes.push(busyMsThisFrame);
      if (busyTimes.length > RING_SIZE) busyTimes.shift();
      particles = particleCount;
    },
  }) as FoundryPerfStats & { _recordFrame: (busyMs: number, particleCount: number) => void };
}
