#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENV_SOURCE="${ROOT_DIR}/.env"

if [[ ! -f "${ENV_SOURCE}" ]]; then
  echo "Missing .env at ${ENV_SOURCE}" >&2
  exit 1
fi

ROOT_DIR="${ROOT_DIR}" ENV_SOURCE="${ENV_SOURCE}" python3 - <<'PY'
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
root = Path(os.environ["ROOT_DIR"])

def get(key: str, default: str = "") -> str:
    return env.get(key, default)

ytdlp_lines = [
    'ALLOWED_ORIGINS: "https://videosync.video,https://content-machine-pbjp.vercel.app"',
    'FILE_TTL_SECONDS: "300"',
    'DOWNLOADS_DIR: "/tmp/downloads"',
    'CLEANUP_INTERVAL_SECONDS: "60"',
    'LOG_LEVEL: "INFO"',
    'PLAYWRIGHT_BROWSERS_PATH: "/tmp/.playwright-browsers"',
    f'GOOGLE_API_KEY: "{get("GOOGLE_API_KEY")}"',
    f'QDRANT_URL: "{get("QDRANT_URL")}"',
    f'QDRANT_API_KEY: "{get("QDRANT_API_KEY")}"',
    f'DATABASE_URL: "{get("DATABASE_URL")}"',
]

video_sync_lines = [
    'RUST_LOG: "info"',
    'FRONTEND_URL: "https://www.videosync.video"',
    'GOOGLE_OAUTH_REDIRECT_URI: "https://www.videosync.video/api/auth/google"',
    'GOOGLE_OAUTH_REDIRECT_URI_AUTH: "https://www.videosync.video/api/auth/google/callback"',
    'APIFY_CIRCUIT_BREAKER_FAILURE_THRESHOLD: "5"',
    'APIFY_CIRCUIT_BREAKER_SUCCESS_THRESHOLD: "2"',
    'APIFY_CIRCUIT_BREAKER_TIMEOUT_SECONDS: "300"',
    'DATABASE_MAX_CONNECTIONS: "20"',
    'CLIPPING_WORKER_CONCURRENCY: "3"',
    'CLIPPING_WORKER_POLL_INTERVAL: "30"',
    'DOWNLOAD_SEMAPHORE_PERMITS: "2"',
    'VECTORIZATION_TIMEOUT_SECONDS: "3600"',
    'FRAME_ANALYSIS_TIMEOUT_SECONDS: "30"',
    'FRAME_ANALYSIS_CONCURRENCY: "3"',
    'MAX_FRAMES_PER_VIDEO: "100"',
    'TEST_ADMIN_EMAIL: "testadmin@videosync.test"',
    f'DATABASE_URL: "{get("DATABASE_URL")}"',
    f'JWT_SECRET: "{get("JWT_SECRET")}"',
    f'GOOGLE_OAUTH_CLIENT_ID: "{get("GOOGLE_OAUTH_CLIENT_ID")}"',
    f'GOOGLE_OAUTH_CLIENT_SECRET: "{get("GOOGLE_OAUTH_CLIENT_SECRET")}"',
    f'ALLOWED_REDIRECT_ORIGINS: "{get("ALLOWED_REDIRECT_ORIGINS")}"',
    f'YOUTUBE_API_KEY: "{get("YOUTUBE_API_KEY")}"',
    f'GOOGLE_API_KEY: "{get("GOOGLE_API_KEY")}"',
    f'GEMINI_API_KEY: "{get("GEMINI_API_KEY")}"',
    f'MANUAL_CLIPPING_GEMINI_API_KEY: "{get("MANUAL_CLIPPING_GEMINI_API_KEY")}"',
    f'VIDEO_GEMINI_API_KEY: "{get("VIDEO_GEMINI_API_KEY")}"',
    f'GEMMA_API_KEY: "{get("GEMMA_API_KEY")}"',
    f'NVIDIA_API_KEY: "{get("NVIDIA_API_KEY")}"',
    f'ANTHROPIC_API_KEY: "{get("ANTHROPIC_API_KEY")}"',
    f'ELEVEN_LABS_API_KEY: "{get("ELEVEN_LABS_API_KEY")}"',
    f'PEXELS_API_KEY: "{get("PEXELS_API_KEY")}"',
    f'VOYAGEAI_API_KEY: "{get("VOYAGEAI_API_KEY")}"',
    f'APIFY_TOKEN: "{get("APIFY_TOKEN")}"',
    f'APIFY_YOUTUBE_CLIENT_ACTOR: "{get("APIFY_YOUTUBE_CLIENT_ACTOR")}"',
    f'TWITCH_TV_CLIENT_ID: "{get("TWITCH_TV_CLIENT_ID")}"',
    f'TWITCH_TV_CLIENT_SECRET: "{get("TWITCH_TV_CLIENT_SECRET")}"',
    f'QDRANT_URL: "{get("QDRANT_URL")}"',
    f'QDRANT_API_KEY: "{get("QDRANT_API_KEY")}"',
    f'ASTRA_DB_API_ENDPOINT: "{get("ASTRA_DB_API_ENDPOINT")}"',
    f'ASTRA_DB_APPLICATION_TOKEN: "{get("ASTRA_DB_APPLICATION_TOKEN")}"',
    f'R2_ACCOUNT_ID: "{get("R2_ACCOUNT_ID")}"',
    f'R2_ACCESS_KEY_ID: "{get("R2_ACCESS_KEY_ID")}"',
    f'R2_SECRET_ACCESS_KEY: "{get("R2_SECRET_ACCESS_KEY")}"',
    f'R2_BUCKET: "{get("R2_BUCKET")}"',
    f'R2_ENDPOINT: "{get("R2_ENDPOINT")}"',
    f'BLENDER_MCP_URL: "{get("BLENDER_MCP_URL")}"',
    f'BLENDER_MCP_API_KEY: "{get("BLENDER_MCP_API_KEY")}"',
    f'YTDLP_API_URL: "{get("YTDLP_API_URL")}"',
    f'YTDLP_PROXY: "{get("YTDLP_PROXY")}"',
    f'YTDLP_COOKIES_B64: "{get("YTDLP_COOKIES_B64")}"',
    f'WEBSHARE_YTDLAPI_API_KEY: "{get("WEBSHARE_YTDLAPI_API_KEY")}"',
    f'WEBSHARE_DOWNLOAD_LINK: "{get("WEBSHARE_DOWNLOAD_LINK")}"',
    f'TEST_ADMIN_PASSWORD: "{get("TEST_ADMIN_PASSWORD")}"',
]

(root / "ytdlp.cloudrun.env.yaml").write_text("\n".join(ytdlp_lines) + "\n")
(root / "video_sync.cloudrun.env.yaml").write_text("\n".join(video_sync_lines) + "\n")

print("Generated:")
print(f"  {root / 'ytdlp.cloudrun.env.yaml'}")
print(f"  {root / 'video_sync.cloudrun.env.yaml'}")
PY
