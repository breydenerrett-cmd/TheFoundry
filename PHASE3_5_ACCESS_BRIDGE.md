# THE FOUNDRY — Phase 3.5: Standalone Access Bridge Investigation

Date: 2026-09-04. Scope: investigate every realistic way a standalone Foundry process could obtain Claude telemetry without a model turn, and build/prove what's actually achievable today. Still no Pixi, no factory rendering.

**Environment caveat, stated up front:** this investigation ran inside a Linux cloud sandbox, not on Brey's own Windows/Mac desktop. Where a claim is empirically tested here (CLI behavior, API docs, real cost), it's marked **TESTED**. Where a claim depends on Claude Desktop specifically (Mac/Windows-only app, not present in this sandbox), it's marked **UNTESTABLE HERE — needs Brey's machine**.

---

## 1. Options investigated, in the requested priority order

### 1. A local endpoint/socket/bridge/command callable without a model turn
**TESTED.** Two real things found:
- `claude agents --json` — prints all active Claude Code sessions (interactive + background) **on the local machine** as a plain JSON array. Confirmed: 0.32s wall-clock, exit before any model call, deterministic, scriptable, "does not require a TTY" per its own `--help` text. This is the best find of the whole investigation — see §2.
- A per-process Unix socket exists (`$CLAUDE_CODE_MESSAGING_SOCKET`, e.g. `/tmp/cc-socks/500.sock`), used internally for `claude attach`/`logs`/`stop`. Its protocol is undocumented. **Deliberately not reverse-engineered or connected to** — depending on an unsupported internal protocol is exactly what §7a says not to do, and `claude agents --json` already gives the same information through a real, stable-looking CLI contract.

Both are **local-machine-only** — they can never see a bridge session on a different machine or a cloud session, by construction (there's no cross-machine transport involved at all).

### 2. Claude Code CLI: deterministic session metadata without model tokens
**TESTED — yes, confirmed.** `claude agents --json` is exactly this. Empirically measured at 0.321s real time, zero API calls, zero tokens. This is the concrete answer to the "cheapest possible polling" question — see §3's cost table. **Now wired into the watcher core as `LocalClaudeObserver` (`src/local.rs`) and proven live (§4).**

### 3. A locally-authenticated Remote Control bridge/proxy, queryable read-only
**TESTED, via documentation — does not exist.** Fetched `code.claude.com/docs/en/claude-code-on-the-web`: Remote Control and cloud sessions run on "Anthropic-managed cloud infrastructure," and `--teleport` "connects through the same Remote Control session infrastructure that cloud sessions use." There is no local daemon or proxy on Brey's machine that owns this data — the source of truth is centrally hosted by Anthropic. `claude mcp list` in this session (a genuine remote/cloud session) shows **zero locally-configured MCP servers**, confirming the `Claude_Code_Remote` tool surface used throughout this project is injected by the session runtime itself, not a normal user-addable local server a companion process could point at.

### 4. An MCP client authenticating directly, without a model in the loop
**TESTED, via documentation — no supported read path exists today.** MCP itself doesn't require an LLM to call a tool — a raw client legitimately could, given valid transport + credentials. The question is whether Brey can *get* such a credential for read access. Found the one public, programmatic, bearer-token-authenticated endpoint in this whole surface: `POST https://api.anthropic.com/v1/claude_code/routines/{id}/fire`. Its own documentation states, verbatim:

> **Token scope: One routine only; no read access.**
> "The bearer token is scoped to a single routine. A compromised token can only trigger that routine; it grants no read access, no access to other routines, and no access to account data."

This is a direct, authoritative, current (2026-09-04) answer: **there is no documented, public, API-key-authenticated read endpoint for session or routine status.** The only public HTTP API in the `claude_code` namespace is write-only (fire a routine).

### 5. A tiny companion/broker process alongside Claude Desktop
**Partially TESTED, partially UNTESTABLE HERE.** Given findings 1-4, a companion process's only route to *estate-wide* (cross-machine, cloud) data is to itself invoke a real, authenticated Claude session/model turn that calls the `Claude_Code_Remote`-equivalent MCP tools — there's no way around that today. Measured the real cost of the cheapest version of that (§3). Whether Claude Desktop specifically exposes an importable local MCP config a companion could read (`claude mcp add-from-claude-desktop` exists and is "Mac and WSL only") is **UNTESTABLE HERE** — no Claude Desktop app runs in this sandbox. Flagged for Brey to check on his own machine; even if it works, it would only surface *locally configured* MCP servers, not a new read credential for the Remote surface itself.

### 6. Best degraded architecture, given 1-5
This is what got built (§4): `local_claude` (zero-token, this-machine sessions) + `git` (zero-token, real repo state) as genuine standalone observers, with `remote_claude` (estate-wide) explicitly marked UNAVAILABLE rather than faked when not manually fed. `.foundry` heartbeat/routine adapters remain designed-but-not-built, per the standing "don't instrument other repos yet" rule — nothing in this investigation changes that.

---

## 2. Which are actually supported (summary table)

| Option | Supported today? | Scope | Model tokens? |
|---|---|---|---|
| `claude agents --json` | **Yes** | Local machine only | **Zero** |
| Local Remote Control proxy | No — doesn't exist | — | — |
| Public read API (`api.anthropic.com/v1/claude_code/...`) | No — write-only, docs say so explicitly | — | — |
| MCP client with a standalone read credential | Unknown/no — no documented path found | — | — |
| Headless `claude -p` calling Remote-surface tools | **Yes, technically** | Estate-wide | **Real, measured, non-trivial (§3)** |
| Claude Desktop local MCP import | Untested here | Local | Unknown |

## 3. Token cost — measured, not estimated

Ran one real minimal headless call from this sandbox:

```
claude -p "Reply with exactly one word: OK" --model claude-haiku-4-5-20251001 --output-format json
```

Result: **$0.0129631**, 10 real input tokens + 42 output tokens, but **5,115 cache-creation tokens + 25,131 cache-read tokens** — the overwhelming majority of the cost is context/system-prompt loading, not the trivial prompt itself. Wall-clock ~3.9s.

Extrapolated (this is exactly what Brey asked to be flagged loudly):

| Poll interval | Calls/day | Cost/day | Cost/month |
|---|---|---|---|
| Every 10s | 8,640 | **~$112** | ~$3,370 |
| Every 60s | 1,440 | **~$18.70** | ~$561 |
| Every 5 min | 288 | ~$3.74 | ~$112 |
| Every 30 min | 48 | ~$0.62 | ~$19 |

**Verdict: any model-turn-based polling approach is financially untenable for continuous ambient telemetry**, exactly as Brey suspected. Even a conservative 5-minute interval costs more per month than the thing it's watching is worth as a background utility. This confirms: estate-wide Remote telemetry cannot be the *primary* polling loop for an always-on dashboard — it can only be refreshed occasionally (e.g. Brey manually, or a very-low-frequency routine, on the order of once every 15-60+ minutes) until a real read API exists.

## 4. Standalone proof — PASS B, demonstrated live

Per the design's honesty rule, **PASS A (full estate-wide autonomous discovery) is not achievable today** — §1 findings 1-4 rule it out with primary-source evidence, not assumption. **PASS B was demonstrated live, end to end, with zero manual data-feeding:**

1. Started a real `claude --bg` session (`0c0eb421`) — not staged, not scripted around Foundry.
2. Foundry (`--no-remote`, `--watch 5`), running continuously with no knowledge of this session in advance, **autonomously discovered it within one 5s poll cycle** via `LocalClaudeObserver`, rendering `[WORKING] (observed)` — `(observed)`, not `(inferred)`, because the background session's real `status: "busy"` field was present (a Phase-3.5 discovery: background sessions carry a genuine activity signal beyond mere liveness, upgrading fidelity accordingly).
3. Ran `claude stop 0c0eb421` externally.
4. Foundry's very next poll showed `[FADING/ENDED] (observed)` — the §16 grace-fade state, live, not simulated.
5. It stayed visibly fading for the ~60s window, then folded into `(1 session(s) ended this run, past the fade window)` — never silently vanishing, exactly per the finding-#5 fix, now proven against a real process lifecycle, not just a unit test.

Simultaneously, `GitObserver` reported this repo's real branch/dirty/last-commit state throughout, and `remote_claude` was correctly and explicitly rendered absent (`--no-remote`) rather than faked — the marquee and SESSIONS header both say `UNKNOWN — capability unavailable`, never a silent zero.

**This is a real Pass B: standalone local observers proven live and autonomous; Remote/cloud capability explicitly and honestly marked unavailable, never faked.**

## 5. Security implications

- `LocalClaudeObserver`/`GitObserver` need no credentials at all — they shell out to already-authenticated local tools (`claude`, `git`) the same way a human would. No token handling, no new attack surface.
- The one real credential type surfaced in this investigation — a routine's `sk-ant-oat01-...` bearer token — is explicitly write-only and single-routine-scoped per Anthropic's own docs, so even if Foundry later used it (e.g. to *trigger* a status-refresh routine, not to read data directly), a leaked token couldn't expose account data. Still: **never commit or log this token**, store it via the OS credential store per §15/§14, exactly as already planned for the eventual real CCR credential.
- No new local socket, port, or IPC surface was added — deliberately avoided connecting to the internal messaging socket found in §1.1.

## 6. Recommended bridge architecture going forward

**Two-tier, honest about the split:**

- **Tier 1 — always-on, zero-cost, real-time-ish:** `LocalClaudeObserver` + `GitObserver` (+ future: heartbeat/`.foundry` adapters once instrumentation is authorized) run on Foundry's own polling cadence (seconds), zero model cost, covering everything on the machine Foundry runs on.
- **Tier 2 — estate-wide, low-frequency, cost-bounded:** a scheduled refresh (Brey manually running the existing live-feed capture, or — if Brey wants this automated later — a routine on a long interval, e.g. hourly, calling the Remote-surface tools once and writing redacted snapshots to the known `live-feed/` contract) updates the cross-machine/cloud picture at a cadence that keeps monthly cost in the tens-of-dollars range, not thousands. **Not built in this phase** — flagged as the natural next increment, gated on Brey deciding an acceptable refresh interval and cost ceiling.
- The renderer already treats these as genuinely separate observers with independent capability tracking — Tier 2 going stale between refreshes will correctly show as `STALE/UNKNOWN`, not silently frozen "fresh," per the finding-#2/#3 fixes.

This does not require Phase 4/Pixi work to adopt — it's a data-layer decision, already compatible with the current architecture.
