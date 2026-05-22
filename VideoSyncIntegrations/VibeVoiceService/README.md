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

### `GET /api/capabilities`

Returns the currently exposed VibeVoice feature set, including:

- available realtime TTS speakers discovered from the vendored repo
- supported ASR feature flags such as diarization, timestamps, hotwords, and context info

### `GET /api/speakers`

Returns the discovered realtime speaker presets, including experimental multilingual voices when those assets are present.

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
  "language": "en",
  "context_info": "Speaker names: Alice, Brian. Topic: crypto payroll infrastructure."
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

## Repo Strategy

VideoSync now vendors the upstream `microsoft/VibeVoice` repository at:

`VideoSyncIntegrations/VibeVoice/`

The service in `VideoSyncIntegrations/VibeVoiceService/` stays as the stable VideoSync-facing API boundary, while the vendored upstream repo provides the actual model/runtime assets and demo entry points.

Current practical status:

- Realtime TTS can be driven from the vendored upstream repo by default.
- Long-form VibeVoice-TTS code was removed upstream by Microsoft, so the service should treat `VibeVoice-Realtime-0.5B` as the supported TTS path unless you provide a different command/runtime.
- ASR is now driven by a small VideoSync adapter that uses the vendored upstream `VibeVoice-ASR` stack by default, while `VIBEVOICE_ASR_COMMAND` remains available as an override.
- The wrapper now exposes discovered realtime speaker presets and richer ASR context injection so downstream VideoSync services can use more of the vendored upstream surface area.

## Default Runtime Behavior

If `VIBEVOICE_TTS_COMMAND` is not provided, the service will try to use the vendored upstream realtime demo automatically:

```bash
python3 VideoSyncIntegrations/VibeVoice/demo/realtime_model_inference_from_file.py \
  --model_path microsoft/VibeVoice-Realtime-0.5B \
  --txt_path <temp input text file> \
  --speaker_name Emma \
  --output_dir <job output dir>
```

Useful environment variables:

- `VIBEVOICE_REPO_DIR`
- `VIBEVOICE_PYTHON_BIN`
- `VIBEVOICE_TTS_MODEL`
- `VIBEVOICE_ASR_MODEL`
- `VIBEVOICE_ASR_DEVICE`
- `VIBEVOICE_OUTPUT_DIR`
- `VIBEVOICE_TTS_COMMAND`
- `VIBEVOICE_ASR_COMMAND`
- `FFMPEG_BIN`

## Production Notes

- Store generated audio and transcripts in R2 so agents can reuse them as assets.
- Keep model downloads on a persistent disk or pre-baked image; do not download weights during request handling.
- Run TTS/transcription jobs asynchronously from Rust for anything longer than quick previews.
- Treat company demos as speculative unless a client explicitly commissions the work.
