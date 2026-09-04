# Phase 4D — Multi-Machine Peer Model

**Status: foundation built — file transport only.** M-01 (this doc's design,
below) is implemented in `watcher-core`: a `foundry-agent` binary runs the
same zero-model-token local observers on a machine, signs each poll's
events with HMAC-SHA256, and publishes them to a shared directory; the main
`foundry` binary verifies, ingests, and renders them in a new `MACHINES`
section. The transport is the one piece deliberately left minimal — see
"What's still open" below.

## Running the agent

```
FOUNDRY_AGENT_KEY=<shared secret> foundry-agent \
    --agent-id pc \
    --transport-dir /path/to/shared/dir \
    --key-id k1 \
    --git-dir /path/to/repo1 [--git-dir /path/to/repo2 ...] \
    [--heartbeat-dir DIR] [--once | --watch 30]
```

- The secret is read ONLY from `FOUNDRY_AGENT_KEY` or `--key-file PATH` —
  never accepted as a bare CLI argument value, never logged.
- `--key-id` is a short, non-secret label naming which shared secret signed
  the bundle (not the secret itself) — the main side maps `key_id` back to
  the matching secret via its own keyring.
- Default is single-shot (poll once, publish once, exit) — `--once` is
  accepted explicitly for clarity at the call site; pass `--watch SECS` to
  keep publishing on an interval.
- Every event's `source` is rewritten to `"<observer>@<agent_id>"` (e.g.
  `local_claude@pc`) before signing — the reducer already treats `source`
  as an arbitrary observer name, so per-machine staleness/capability
  tracking works with zero changes to the reducer or renderer.

Main side:

```
foundry --agents-dir /path/to/shared/dir \
    --agent-keys k1=/path/to/keyfile \
    [--agent-ttl 120] \
    [other usual foundry flags...]
```

(or `--agent-key-id k1` paired with `FOUNDRY_AGENT_KEY` in the main
process's own environment, for the single-key case.) An agent that goes
quiet past `--agent-ttl` (default 120s) renders `UNREACHABLE` in the
`MACHINES` section; its sessions are not force-expired specially — they age
out honestly through the ordinary per-session TTL path once the agent
simply stops supplying fresh events, the same as any other observer going
quiet.

## What's still open

- **HTTP transport.** `FileTransport` (a shared directory both processes can
  read/write — works today for co-located machines, e.g. a synced folder or
  a shared drive) is the only `transport::Publisher`/`Receiver`
  implementation built. An authenticated HTTP push (agent -> main's LAN
  address) remains the recommended next implementation per the design
  below — `Publisher`/`Receiver` is the seam it slots into; nothing in
  `agents.rs`, the reducer, or the renderer should need to change.
- **Key provisioning UX.** Keys are currently plain files a human copies
  around by hand (`--key-file` / `--agent-keys id=path`). No key rotation,
  no OS credential-store integration (§15's original recommendation), no
  provisioning flow for "add a new machine." Fine for a single operator
  running two machines; not yet a real onboarding story.
- Everything else originally flagged below (which machines, which network,
  a real multi-writer eventlog/persistence story if agents and main ever
  run as genuinely independent long-lived processes with their own state)
  is still open.

---

## Original design (below), now implemented as described above

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
