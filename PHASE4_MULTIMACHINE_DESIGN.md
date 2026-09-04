# Phase 4D — Multi-Machine Peer Model (design only, not built)

## Problem

`LocalClaudeObserver`/`GitObserver` only ever see the machine Foundry runs on. Brey runs Claude sessions across multiple machines (PC, Mac). Each machine can produce its own zero-cost local observations (§Phase 3.5/4A) — the gap is getting them to one place without a model turn.

## Design: `Foundry Agent`, one per machine

```
[Foundry Agent — PC]  --publishes normalized events-->  \
[Foundry Agent — Mac] --publishes normalized events--> main Foundry (reducer + renderer)
```

- **Foundry Agent** = the SAME `local_claude` + `git` (+ `.foundry` heartbeat) observers already built, running as a thin always-on process on each machine. Zero model tokens — identical to what already runs today, just deployed per-machine.
- **Transport: deliberately unspecified/pluggable**, not baked into the observer/reducer contract (same principle as `RemoteClaudeObserver`'s adapter boundary — the DATA shape is fixed, the delivery mechanism isn't). Candidates, not decided:
  - A small authenticated HTTP push (agent → main Foundry's local LAN address) — simplest, works today with `reqwest`/`tiny_http`.
  - A pull model (main Foundry polls each agent's `/events` endpoint) — simpler main-side logic, needs each agent reachable.
  - A shared file (synced folder, e.g. Dropbox/Syncthing) — zero network code, adds sync latency and a third-party dependency.
  - Recommendation when this gets built: **authenticated HTTP push**, agent → main, since it matches the existing poll-and-normalize model most closely and needs no new sync tooling.
- **Authentication:** a per-agent shared secret (generated once, stored via OS credential store per §15, never logged) — NOT the CCR/Remote credential, a separate one scoped only to "this machine may publish its own local observations." A compromised agent token can only inject fabricated LOCAL data for that one machine, never touch Remote/cloud data or other machines.
- **What each agent publishes:** the exact same `schema::Event` shape already defined (§8) — no new wire format. The main Foundry's reducer already treats `source` as an arbitrary observer name; a remote agent's events just carry `source: "local_claude@pc-hostname"` etc., so degradation/staleness/capability tracking work unchanged.
- **Machine identity in the floor:** each machine becomes its own row in a new `MACHINES` section (not built) showing last-heartbeat age per agent — the same STALE-not-IDLE honesty rule applies: an agent that stops publishing must show as unreachable, never silently drop off.

## Why not built now

Phase 4's stop gate is about proving the SINGLE-machine zero-cost model works end-to-end first (done — see the live demo). Splitting into a real network protocol before that's solid would be premature; the transport is explicitly designed to be swappable later without touching the schema, reducer, or renderer. Building it now would also mean designing real authentication/network-exposure decisions Brey hasn't been asked about yet (which machines, which network, inbound-vs-outbound).
