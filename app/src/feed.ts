// Isolates data acquisition from rendering. Today: fixture-only, read from a
// static JSON file bundled with the app (no network fetch beyond the local
// dev/preview server serving its own static asset). Later: a live `/state`
// WebSocket or Tauri event stream replaces `loadFloorState` below — nothing
// outside this file should need to change (renderer only depends on
// `FloorState` from state.ts).

import type { FloorState } from "./state";

const DEFAULT_FIXTURE = "/fixtures/floor.json";

function fixtureUrl(): string {
  const params = new URLSearchParams(window.location.search);
  const requested = params.get("fixture");
  if (requested && requested.length > 0) {
    // Only ever resolve fixture names against our own public/fixtures dir —
    // never treat the query param as an arbitrary URL.
    const safe = requested.replace(/[^a-zA-Z0-9._-]/g, "");
    return `/fixtures/${safe}`;
  }
  return DEFAULT_FIXTURE;
}

export async function loadFloorState(): Promise<FloorState> {
  const url = fixtureUrl();
  const res = await fetch(url);
  if (!res.ok) {
    throw new Error(`failed to load fixture ${url}: ${res.status}`);
  }
  return (await res.json()) as FloorState;
}
