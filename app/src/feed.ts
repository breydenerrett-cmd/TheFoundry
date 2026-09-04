// Isolates data acquisition from rendering. Three sources, selected by the
// `?feed=` URL query:
//   `?feed=fixture[:name]` — static JSON from public/fixtures/ (also
//     reachable via the legacy `?fixture=name` param, kept for the existing
//     fixture-driven test suite).
//   `?feed=http[:base]`    — poll a live watcher-core `--serve` HTTP
//     server's `GET /state` (default base `http://127.0.0.1:8790`), every
//     2s, with a 1.5s per-request timeout via AbortController. Optional
//     `?token=` is sent as `X-Foundry-Token`.
//   `?feed=file:`          — reserved for the future Tauri file-watch
//     bridge. Deliberately unimplemented: throws rather than faking data.
//
// With NO `?feed=`/`?fixture=` query at all, the app tries the live http
// feed and, if unreachable, renders nothing loaded — never a fixture that
// could be mistaken for a real, live estate. A missing/unreachable watcher
// must always show as WATCHER DOWN, never a quietly-substituted demo.
//
// Nothing outside this file should need to change when the real transport
// changes — the renderer only ever depends on `FloorState` from state.ts.

import type { FloorState } from "./state";

const DEFAULT_FIXTURE = "floor.json";
const DEFAULT_HTTP_BASE = "http://127.0.0.1:8790";
const POLL_INTERVAL_MS = 2000;
const FETCH_TIMEOUT_MS = 1500;

/** Consecutive poll misses (once we HAVE seen a good fetch) before the feed
 *  is considered `down` rather than merely a blip — mission's ">3 polls". A
 *  feed that has NEVER succeeded goes `down` on its very first failed
 *  attempt (the "closed port -> down immediately" case). */
const MAX_MISSES_AFTER_OK = 3;
/** `generated_at` older than this (ms) counts as stale even if polls are
 *  succeeding — the watcher itself has stopped producing fresh state. */
const STALE_GENERATED_AT_MS = 30_000;
/** `generated_at` not changing for this long (ms) — our seq-advancement
 *  proxy, since the `/state` payload itself carries no seq field (only
 *  `/health` does) — counts as stale ("seq frozen"). */
const STALE_UNCHANGED_MS = 60_000;

export type FeedKind = "fixture" | "http" | "file";
export type FeedLiveness = "live" | "stale" | "down" | "fixture";

export interface FeedStatus {
  kind: FeedKind;
  liveness: FeedLiveness;
  /** ms since epoch of the last successful fetch, or null if never. */
  lastFetchOkAt: number | null;
  /** `generated_at` from the most recently fetched state, if any. */
  generatedAt: string | null;
  /** ms since epoch that `generatedAt` last actually changed value — our
   *  seq-advancement proxy. */
  lastChangedAt: number | null;
  consecutiveMisses: number;
}

export type FeedListener = (state: FloorState | null, status: FeedStatus) => void;

export interface FeedHandle {
  stop(): void;
}

function parseFeedQuery(): { kind: FeedKind | null; rest: string } {
  const params = new URLSearchParams(window.location.search);
  const raw = params.get("feed");
  if (raw) {
    const idx = raw.indexOf(":");
    const kind = (idx === -1 ? raw : raw.slice(0, idx)) as FeedKind;
    const rest = idx === -1 ? "" : raw.slice(idx + 1);
    if (kind === "fixture" || kind === "http" || kind === "file") {
      return { kind, rest };
    }
  }
  // Legacy `?fixture=name` param — explicit fixture request.
  const legacyFixture = params.get("fixture");
  if (legacyFixture) {
    return { kind: "fixture", rest: legacyFixture };
  }
  return { kind: null, rest: "" };
}

function fixtureUrl(name: string): string {
  const safe = (name || DEFAULT_FIXTURE).replace(/[^a-zA-Z0-9._-]/g, "");
  return `/fixtures/${safe || DEFAULT_FIXTURE}`;
}

function httpToken(): string | null {
  return new URLSearchParams(window.location.search).get("token");
}

async function fetchJson(url: string, headers: Record<string, string>): Promise<unknown> {
  const controller = new AbortController();
  const timer = window.setTimeout(() => controller.abort(), FETCH_TIMEOUT_MS);
  try {
    const res = await fetch(url, { headers, signal: controller.signal, cache: "no-store" });
    if (!res.ok) {
      throw new Error(`${url}: ${res.status}`);
    }
    return await res.json();
  } finally {
    window.clearTimeout(timer);
  }
}

/** Fixture mode: fetch once, report `liveness: "fixture"` forever — a
 *  fixture can never be mistaken for a live or down estate. */
function startFixtureFeed(name: string, listener: FeedListener): FeedHandle {
  let stopped = false;
  fetch(fixtureUrl(name))
    .then((res) => {
      if (stopped) return null;
      if (!res.ok) throw new Error(`failed to load fixture ${name}: ${res.status}`);
      return res.json();
    })
    .then((json) => {
      if (stopped || !json) return;
      listener(json as FloorState, {
        kind: "fixture",
        liveness: "fixture",
        lastFetchOkAt: Date.now(),
        generatedAt: (json as FloorState).generated_at ?? null,
        lastChangedAt: Date.now(),
        consecutiveMisses: 0,
      });
    })
    .catch((e) => {
      if (stopped) return;
      // eslint-disable-next-line no-console
      console.error("fixture feed failed to load", e);
      listener(null, {
        kind: "fixture",
        liveness: "fixture",
        lastFetchOkAt: null,
        generatedAt: null,
        lastChangedAt: null,
        consecutiveMisses: 0,
      });
    });
  return {
    stop() {
      stopped = true;
    },
  };
}

/** file: — reserved for the Tauri desktop shell's local file-watch bridge.
 *  Deliberately unimplemented rather than faked. */
function startFileFeed(): FeedHandle {
  throw new Error("feed=file: not wired — the Tauri file-watch bridge does not exist yet");
}

function startHttpFeed(base: string, listener: FeedListener): FeedHandle {
  const root = (base || DEFAULT_HTTP_BASE).replace(/\/+$/, "");
  const token = httpToken();
  const headers: Record<string, string> = token ? { "X-Foundry-Token": token } : {};

  let stopped = false;
  let lastFetchOkAt: number | null = null;
  let generatedAt: string | null = null;
  let lastChangedAt: number | null = null;
  let consecutiveMisses = 0;
  let lastState: FloorState | null = null;

  function computeLiveness(now: number): FeedLiveness {
    if (lastFetchOkAt === null) return "down";
    const missThreshold = MAX_MISSES_AFTER_OK;
    if (consecutiveMisses > missThreshold) return "down";
    const genAgeMs = generatedAt ? now - Date.parse(generatedAt) : null;
    if (genAgeMs !== null && Number.isFinite(genAgeMs) && genAgeMs > STALE_GENERATED_AT_MS) {
      return "stale";
    }
    if (lastChangedAt !== null && now - lastChangedAt > STALE_UNCHANGED_MS) {
      return "stale";
    }
    return "live";
  }

  function emit(): void {
    const now = Date.now();
    listener(lastState, {
      kind: "http",
      liveness: computeLiveness(now),
      lastFetchOkAt,
      generatedAt,
      lastChangedAt,
      consecutiveMisses,
    });
  }

  async function poll(): Promise<void> {
    try {
      const json = (await fetchJson(`${root}/state`, headers)) as FloorState;
      if (stopped) return;
      // The watcher's httpd starts serving `/state` the instant its listener
      // binds, before the first poll cycle has populated it — briefly the
      // literal placeholder `{}`. Treat anything that isn't a real
      // FloorState shape as a miss rather than handing bad data downstream.
      if (!json || !Array.isArray(json.sessions)) {
        throw new Error("watcher /state not ready yet (placeholder response)");
      }
      consecutiveMisses = 0;
      lastFetchOkAt = Date.now();
      lastState = json;
      const nextGeneratedAt = json.generated_at ?? null;
      if (nextGeneratedAt !== generatedAt) {
        generatedAt = nextGeneratedAt;
        lastChangedAt = lastFetchOkAt;
      }
    } catch {
      if (stopped) return;
      consecutiveMisses += 1;
    }
    emit();
  }

  // Fire immediately, then on the interval.
  void poll();
  const iv = window.setInterval(() => void poll(), POLL_INTERVAL_MS);

  return {
    stop() {
      stopped = true;
      window.clearInterval(iv);
    },
  };
}

/** Starts the feed selected by the URL query and calls `listener` on every
 *  poll (successful or not) with the latest known `FloorState` (or `null`
 *  if nothing has ever loaded) and a `FeedStatus` describing liveness.
 *  Returns a handle to stop polling (e.g. on component unmount). */
export function startFeed(listener: FeedListener): FeedHandle {
  const { kind, rest } = parseFeedQuery();
  if (kind === "fixture") return startFixtureFeed(rest, listener);
  if (kind === "file") return startFileFeed();
  // kind === "http", or no query at all — default: try http, never fall
  // back to a fixture.
  return startHttpFeed(kind === "http" ? rest : "", listener);
}
