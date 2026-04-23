from __future__ import annotations

import base64
import os
import tempfile
from pathlib import Path
from typing import Any
from uuid import uuid4

import httpx
from fastapi import FastAPI, HTTPException
from pydantic import BaseModel, Field

try:
    from videosync_vibevoice_client import VibeVoiceRuntime
except Exception as exc:  # pragma: no cover - import guard for lightweight deploys
    VibeVoiceRuntime = None  # type: ignore[assignment]
    IMPORT_ERROR = str(exc)
else:
    IMPORT_ERROR = None


app = FastAPI(
    title="VideoSync VibeVoice Service",
    version="0.1.0",
    description="Shared narration and transcription service for VideoSync agents.",
)


class TtsRequest(BaseModel):
    text: str = Field(..., min_length=1)
    speaker: str | None = "Emma"
    format: str = "wav"
    job_id: str | None = None
    metadata: dict[str, Any] = Field(default_factory=dict)


class TranscribeRequest(BaseModel):
    audio_url: str
    hotwords: list[str] = Field(default_factory=list)
    language: str | None = None
    job_id: str | None = None
    metadata: dict[str, Any] = Field(default_factory=dict)


def runtime() -> Any:
    if VibeVoiceRuntime is None:
        raise HTTPException(
            status_code=501,
            detail=f"VibeVoice runtime is not installed or failed to import: {IMPORT_ERROR}",
        )
    return VibeVoiceRuntime.from_env()


@app.get("/health")
def health() -> dict[str, Any]:
    return {
        "ok": True,
        "service": "videosync-vibevoice",
        "runtime_available": VibeVoiceRuntime is not None,
        "import_error": IMPORT_ERROR,
        "tts_model": os.getenv("VIBEVOICE_TTS_MODEL", ""),
        "asr_model": os.getenv("VIBEVOICE_ASR_MODEL", ""),
    }


@app.post("/api/tts")
async def text_to_speech(req: TtsRequest) -> dict[str, Any]:
    if not req.text.strip():
        raise HTTPException(status_code=400, detail="text is required")

    result = await runtime().text_to_speech(
        text=req.text,
        speaker=req.speaker or "Emma",
        output_format=req.format,
        job_id=req.job_id or str(uuid4()),
        metadata=req.metadata,
    )
    return {"success": True, **result}


@app.post("/api/transcribe")
async def transcribe(req: TranscribeRequest) -> dict[str, Any]:
    if not req.audio_url.startswith(("https://", "http://")):
        raise HTTPException(status_code=400, detail="audio_url must be http(s)")

    with tempfile.TemporaryDirectory(prefix="videosync-vibevoice-") as tmp:
        audio_path = Path(tmp) / "input_audio"
        async with httpx.AsyncClient(timeout=120.0, follow_redirects=True) as client:
            response = await client.get(req.audio_url)
            response.raise_for_status()
            audio_path.write_bytes(response.content)

        result = await runtime().transcribe(
            audio_path=audio_path,
            hotwords=req.hotwords,
            language=req.language,
            job_id=req.job_id or str(uuid4()),
            metadata=req.metadata,
        )
    return {"success": True, **result}


@app.post("/api/tts/base64")
async def text_to_speech_base64(req: TtsRequest) -> dict[str, Any]:
    result = await text_to_speech(req)
    local_path = result.get("local_path")
    if not local_path:
        raise HTTPException(status_code=501, detail="Runtime did not return a local_path")

    audio_bytes = Path(local_path).read_bytes()
    return {
        "success": True,
        "provider": result.get("provider", "vibevoice"),
        "format": req.format,
        "audio_base64": base64.b64encode(audio_bytes).decode("ascii"),
        "duration_seconds": result.get("duration_seconds"),
    }
