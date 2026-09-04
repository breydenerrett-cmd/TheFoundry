#!/usr/bin/env bash
# THE FOUNDRY — run the real thing: build the watcher, serve its live
# `/state` feed, and start the app pointed at it. One command, no fixtures.
#
# Usage: scripts/dev.sh
#
# Ctrl-C stops both the watcher and the app dev server.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WATCHER_DIR="$ROOT_DIR/watcher-core"
APP_DIR="$ROOT_DIR/app"
LOG_DIR="$ROOT_DIR/.foundry-state"
STATE_JSON="$APP_DIR/public/state.json"
SERVE_ADDR="127.0.0.1:8790"

mkdir -p "$LOG_DIR"

echo "==> building watcher-core"
(cd "$WATCHER_DIR" && cargo build -q)

WATCHER_BIN="$WATCHER_DIR/target/debug/foundry"

echo "==> starting watcher-core --serve $SERVE_ADDR (log dir: $LOG_DIR)"
"$WATCHER_BIN" \
  --no-remote \
  --git-dir "$ROOT_DIR" \
  --state-json "$STATE_JSON" \
  --serve "$SERVE_ADDR" \
  --watch 2 \
  --log-dir "$LOG_DIR" \
  --bay-map "$ROOT_DIR/foundry.bays.toml" &
WATCHER_PID=$!

cleanup() {
  echo "==> stopping watcher-core (pid $WATCHER_PID)"
  kill "$WATCHER_PID" 2>/dev/null || true
  wait "$WATCHER_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

echo "==> waiting for watcher-core to come up"
for _ in $(seq 1 50); do
  if curl -fsS "http://$SERVE_ADDR/health" >/dev/null 2>&1; then
    break
  fi
  sleep 0.2
done

echo "==> starting app dev server (npm run dev)"
echo "    open http://localhost:5173/  — feed defaults to http://127.0.0.1:8790"
(cd "$APP_DIR" && npm run dev)
