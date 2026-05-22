#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOG_FILE="${1:-/tmp/video_editor_startup.log}"
SECONDS_TO_CAPTURE="${2:-20}"

cd "$ROOT_DIR"

set -a
source .env
set +a

export PATH="$ROOT_DIR/VideoSyncIntegrations/YTDLPAPI/env/bin:$PATH"

: > "$LOG_FILE"
("$ROOT_DIR/target/debug/video_editor" >"$LOG_FILE" 2>&1) &
PID=$!

sleep "$SECONDS_TO_CAPTURE"
kill "$PID" >/dev/null 2>&1 || true
sleep 1

cat "$LOG_FILE"
