#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

export PATH="$ROOT_DIR/VideoSyncIntegrations/YTDLPAPI/env/bin:$PATH"

set -a
source .env
set +a

exec "$@"
