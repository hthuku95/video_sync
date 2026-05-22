#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENV_SOURCE="${ROOT_DIR}/.env"
OUT_FILE="${ROOT_DIR}/blender_mcp.gce.env"

if [[ ! -f "${ENV_SOURCE}" ]]; then
  echo "Missing ${ENV_SOURCE}" >&2
  exit 1
fi

ROOT_DIR="${ROOT_DIR}" ENV_SOURCE="${ENV_SOURCE}" OUT_FILE="${OUT_FILE}" python3 - <<'PY'
from pathlib import Path
import os

def parse_env(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for raw_line in path.read_text().splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        values[key.strip()] = value.strip()
    return values

env = parse_env(Path(os.environ["ENV_SOURCE"]))

def first(*keys: str, default: str = "") -> str:
    for key in keys:
        value = env.get(key, "")
        if value:
            return value
    return default

lines = [
    f"ANTHROPIC_API_KEY={first('ANTHROPIC_API_KEY')}",
    f"GEMINI_API_KEY={first('GEMINI_API_KEY')}",
    f"VIDEO_GEMINI_API_KEY={first('VIDEO_GEMINI_API_KEY')}",
    f"BLENDER_GEMINI_API_KEY={first('BLENDER_GEMINI_API_KEY')}",
    f"NVIDIA_API_KEY={first('NVIDIA_API_KEY')}",
    f"R2_ACCOUNT_ID={first('R2_ACCOUNT_ID')}",
    f"R2_ACCESS_KEY_ID={first('R2_ACCESS_KEY_ID')}",
    f"R2_SECRET_ACCESS_KEY={first('R2_SECRET_ACCESS_KEY')}",
    "R2_BUCKET_NAME=blender-outputs",
    f"MCP_API_KEY={first('BLENDER_MCP_API_KEY', 'MCP_API_KEY')}",
    "PORT=8000",
    "LLM_PROVIDER=gemini",
    f"LANGGRAPH_POSTGRES_URL={first('LANGGRAPH_POSTGRES_URL', 'NEON_DATABASE_URL', 'DATABASE_URL')}",
    f"DATABASE_URL={first('DATABASE_URL')}",
    f"VIBEVOICE_SERVICE_URL={first('VIBEVOICE_SERVICE_URL', default='https://videosync-vibevoice-723463981172.us-central1.run.app')}",
    f"JOB_QUEUE_WORKERS={first('JOB_QUEUE_WORKERS', default='1')}",
    f"JOB_TIMEOUT_SECS={first('JOB_TIMEOUT_SECS', default='1500')}",
    f"RATE_LIMIT_ENABLED={first('RATE_LIMIT_ENABLED', default='true')}",
    f"RATE_LIMIT_RPS={first('RATE_LIMIT_RPS', default='5')}",
    f"RATE_LIMIT_CAPACITY={first('RATE_LIMIT_CAPACITY', default='10')}",
]

Path(os.environ["OUT_FILE"]).write_text("\n".join(lines) + "\n")
print(f"Wrote {os.environ['OUT_FILE']}")
PY
