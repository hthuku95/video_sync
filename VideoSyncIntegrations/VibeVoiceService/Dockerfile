FROM python:3.11-slim

ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    VIBEVOICE_OUTPUT_DIR=/tmp/videosync-vibevoice \
    VIBEVOICE_REPO_DIR=/app/VideoSyncIntegrations/VibeVoice

WORKDIR /app/VideoSyncIntegrations/VibeVoiceService

RUN apt-get update \
    && apt-get install -y --no-install-recommends ffmpeg \
    && rm -rf /var/lib/apt/lists/*

COPY VibeVoiceService/requirements.txt ./requirements.txt
RUN pip install --no-cache-dir -r requirements.txt

COPY VibeVoice /app/VideoSyncIntegrations/VibeVoice
RUN pip install --no-cache-dir /app/VideoSyncIntegrations/VibeVoice

COPY VibeVoiceService /app/VideoSyncIntegrations/VibeVoiceService

EXPOSE 8015

CMD ["/bin/sh", "-c", "uvicorn app:app --host 0.0.0.0 --port ${PORT:-8015}"]
