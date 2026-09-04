# Phase 4B — `.foundry/events.jsonl` Heartbeat Contract

**Status: contract + observer built and tested (`HeartbeatObserver`, 3 tests). NOT applied to the sports scripts themselves** — see "Why not applied" below.

## Contract

One JSON object per line, append-only, at `<repo>/.foundry/events.jsonl`:

```json
{"component":"forward_capture","event":"end","status":"ok","ts":"2026-09-04T07:15:03Z","artifact":"3 files changed"}
{"component":"daily_loop","event":"end","status":"escalate","ts":"2026-09-04T10:07:00Z","error":"credit floor hit"}
```

| Field | Required | Values | Rule |
|---|---|---|---|
| `component` | yes | short stable id (`forward_capture`, `daily_loop`, `monitor_remote`, `test_runner`) | |
| `event` | yes | `start` \| `end` \| `status` \| `escalate` | |
| `status` | yes | `ok` \| `degraded` \| `down` \| `escalate` | |
| `ts` | yes | RFC3339 | |
| `artifact` | no | short descriptive string (file count, commit sha, URL) | **never raw output/file contents** |
| `error` | no | one line | **never a stack trace, secret, or transcript excerpt** |

Foundry's `HeartbeatObserver` is read-only and redacts every free-text field again anyway (defense in depth, same rule already applied to `--audit`'s raw preview) — but the writer must still not put anything sensitive there in the first place.

## Ready-to-paste snippets (not applied — see below)

A tiny bash helper, drop into each script:

```bash
foundry_beat() {  # component event status [artifact] [error]
  mkdir -p .foundry
  python3 - "$1" "$2" "$3" "${4:-}" "${5:-}" <<'PY' >> .foundry/events.jsonl
import json, sys, datetime
c, e, s, a, err = sys.argv[1:6]
rec = {"component": c, "event": e, "status": s, "ts": datetime.datetime.now(datetime.UTC).isoformat()}
if a: rec["artifact"] = a
if err: rec["error"] = err
print(json.dumps(rec))
PY
}
```

Usage at the natural start/end points of each script, e.g. in `forward_capture.sh`:

```bash
foundry_beat forward_capture start ok
# ... existing logic ...
foundry_beat forward_capture end ok "watch+umpires+dense committed"
```

And on an ESCALATE line already detected by the script's own logic:

```bash
foundry_beat forward_capture end escalate "credit floor" "$ESCALATE_LINE"
```

Equivalent one-liner for `monitor_remote.sh`'s existing up/down/degraded classification, and for `scripts/test_parallel.py` (Python — just call a `foundry_beat(...)` helper function instead of the bash one, same JSON shape).

## Why this was not applied to the actual scripts

`scripts/forward_capture.sh`, `daily_loop.sh`, `monitor_remote.sh`, and `test_parallel.py` **do not exist on this branch** — they live on `claude/sports-betting-analysis-review-g1o0co`, a separate, actively-developed branch with its own autonomous worker lanes running against it right now. That branch's own scripts carry an explicit warning: *"This file may be mid-execution on a shared checkout... deploy with a rename into place, never an in-place edit."*

Editing a live-infrastructure script on a different, actively-running branch from this design/dashboard branch is outside this session's mandate and carries real risk of colliding with in-flight work. The contract and snippets above are ready to paste in on that branch (or by a session working there) whenever Brey wants it done — this is a one-line addition per script, not a redesign.

## What's proven instead

`HeartbeatObserver` was built and tested against synthetic `.foundry/events.jsonl` files matching this exact contract: reads real lines, redacts a path embedded in an `error` field, tolerates one malformed line without losing the rest of the file, and degrades honestly (not a panic) when the file doesn't exist yet. See `ops-room/watcher-core/tests/heartbeat_observer.rs`.
