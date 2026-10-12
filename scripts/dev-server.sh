#!/usr/bin/env bash
# Start (or restart) a local Kindly server for development.
# Usage: scripts/dev-server.sh [--reset] [--live] [--port 8787]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DATA_DIR="${KINDLY_DATA_DIR:-$ROOT/data}"
PID_FILE="$DATA_DIR/server.pid"
PORT="${PORT:-8787}"

mkdir -p "$DATA_DIR"

if [[ -f "$PID_FILE" ]]; then
  OLD_PID="$(cat "$PID_FILE")"
  if kill -0 "$OLD_PID" 2>/dev/null; then
    kill "$OLD_PID" 2>/dev/null || true
    sleep 0.5
  fi
  rm -f "$PID_FILE"
fi

MODE="sim"
EXTRA_ARGS=()
for arg in "$@"; do
  case "$arg" in
    --live) MODE="live" ;;
    --reset) EXTRA_ARGS+=("--reset") ;;
    *) EXTRA_ARGS+=("$arg") ;;
  esac
done

cd "$ROOT"
KINDLY_DATA_DIR="$DATA_DIR" KINDLY_PORT="$PORT" \
  ./target/debug/kindly serve --paypal "$MODE" "${EXTRA_ARGS[@]}" \
  > "$DATA_DIR/server.log" 2>&1 &
echo $! > "$PID_FILE"

for _ in $(seq 1 40); do
  if curl -sf "http://localhost:$PORT/api/health" > /dev/null 2>&1; then
    echo "Kindly is up on http://localhost:$PORT (pid $(cat "$PID_FILE"), mode $MODE)"
    echo "logs: $DATA_DIR/server.log"
    exit 0
  fi
  sleep 0.25
done

echo "server did not come up; last log lines:" >&2
tail -20 "$DATA_DIR/server.log" >&2
exit 1
