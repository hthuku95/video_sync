#!/bin/bash
set -e

# Start yt-dlp service (if available)
if [ -d /opt/media/ytdlp ]; then
    cd /opt/media/ytdlp && python3 -m uvicorn app.main:app --host 0.0.0.0 --port 8001 &
fi

# Start HyperFrames renderer (if available)
if [ -d /opt/hyperframes/renderer-service ]; then
    cd /opt/hyperframes/renderer-service && npm start &
fi

# Start Rust API (main process)
exec video_editor
