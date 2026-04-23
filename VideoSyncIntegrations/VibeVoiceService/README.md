# VideoSync VibeVoice Service

HTTP microservice wrapper for VibeVoice-based narration and transcription.

This service is intentionally separate from the Rust backend and BlenderMCPServer so the same voice system can serve:

- Blender website-to-video explainers and product walkthroughs.
- Manim/LaTeX educational videos.
- Agentic video editing and generation jobs.
- Monetizable transcription/diarization offers for podcasts, calls, webinars, and creator content.

## Why A Separate Service?

VibeVoice models are Python/GPU-oriented and can be heavier than the Rust web API. Running them as a microservice keeps long audio jobs asynchronous, lets Render/GPU workers scale separately, and prevents Blender renders from blocking voice generation.

## API Contract

### `GET /health`

Returns service readiness and which providers are installed.

### `POST /api/tts`

Generate narration from text.

```json
{
  "text": "Welcome to the product walkthrough...",
  "speaker": "Emma",
  "format": "wav",
  "job_id": "optional-correlation-id",
  "metadata": {
    "pipeline": "blender",
    "delivery_id": "..."
  }
}
```

Returns:

```json
{
  "success": true,
  "provider": "vibevoice",
  "audio_url": "https://r2.example/audio.wav",
  "duration_seconds": 34.2
}
```

### `POST /api/transcribe`

Transcribe audio by URL. The intended production path is VibeVoice-ASR for long-form structured transcription with speaker/time/content output.

```json
{
  "audio_url": "https://r2.example/podcast.mp3",
  "hotwords": ["VideoSync", "USDC", "Base"],
  "language": "en"
}
```

Returns:

```json
{
  "success": true,
  "provider": "vibevoice-asr",
  "text": "Speaker 1: ...",
  "segments": [
    { "speaker": "Speaker 1", "start": 0.0, "end": 3.8, "text": "..." }
  ]
}
```

## Local Run

```bash
python -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt
uvicorn app:app --host 0.0.0.0 --port 8015
```

Without VibeVoice model packages installed, endpoints return `501` with a clear setup message. This keeps the service deployable before GPU/model provisioning is finished.

## Production Notes

- Store generated audio and transcripts in R2 so agents can reuse them as assets.
- Keep model downloads on a persistent disk or pre-baked image; do not download weights during request handling.
- Run TTS/transcription jobs asynchronously from Rust for anything longer than quick previews.
- Treat company demos as speculative unless a client explicitly commissions the work.
