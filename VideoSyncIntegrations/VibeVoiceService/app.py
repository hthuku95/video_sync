from __future__ import annotations

import asyncio
import base64
import logging
import os
import subprocess
import tempfile
import time
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

logger = logging.getLogger(__name__)
_runtime_lock = asyncio.Lock()


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
    context_info: str | None = None
    job_id: str | None = None
    metadata: dict[str, Any] = Field(default_factory=dict)


def runtime() -> Any:
    if VibeVoiceRuntime is None:
        raise HTTPException(
            status_code=501,
            detail=f"VibeVoice runtime is not installed or failed to import: {IMPORT_ERROR}",
        )
    return VibeVoiceRuntime.from_env()


def maybe_convert_audio(local_path: Path, output_format: str) -> Path:
    requested = (output_format or "wav").lower()
    current = local_path.suffix.lstrip(".").lower()
    if requested == current or not requested:
        return local_path

    converted_path = local_path.with_suffix(f".{requested}")
    command = [
        os.getenv("FFMPEG_BIN", "ffmpeg"),
        "-y",
        "-i",
        str(local_path),
        str(converted_path),
    ]
    try:
        subprocess.run(
            command,
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
    except FileNotFoundError as exc:
        raise HTTPException(
            status_code=501,
            detail=(
                f"Requested output format '{requested}' requires ffmpeg conversion, but ffmpeg "
                f"is not available: {exc}"
            ),
        ) from exc
    except subprocess.CalledProcessError as exc:
        raise HTTPException(
            status_code=500,
            detail=(
                f"ffmpeg conversion to '{requested}' failed: "
                f"{exc.stderr or exc.stdout or str(exc)}"
            ),
        ) from exc

    return converted_path


def _tts_job_id(req: TtsRequest) -> str:
    return req.job_id or str(uuid4())


def _transcribe_job_id(req: TranscribeRequest) -> str:
    return req.job_id or str(uuid4())


def _error_detail(kind: str, *, job_id: str, exc: Exception) -> str:
    return f"{kind} failed for job {job_id}: {type(exc).__name__}: {exc}"


@app.get("/health")
def health() -> dict[str, Any]:
    runtime_obj = VibeVoiceRuntime.from_env() if VibeVoiceRuntime is not None else None
    capabilities = runtime_obj.capabilities() if runtime_obj is not None else {}
    return {
        "ok": True,
        "service": "videosync-vibevoice",
        "runtime_available": VibeVoiceRuntime is not None,
        "import_error": IMPORT_ERROR,
        "tts_model": os.getenv("VIBEVOICE_TTS_MODEL", ""),
        "asr_model": os.getenv("VIBEVOICE_ASR_MODEL", ""),
        "speaker_count": len(capabilities.get("tts", {}).get("available_speakers", [])),
        "capabilities": capabilities,
    }


@app.get("/api/capabilities")
def capabilities() -> dict[str, Any]:
    return {
        "success": True,
        **runtime().capabilities(),
    }


@app.get("/api/speakers")
def speakers() -> dict[str, Any]:
    runtime_obj = runtime()
    capabilities = runtime_obj.capabilities()
    return {
        "success": True,
        "speakers": capabilities.get("tts", {}).get("available_speakers", []),
    }


@app.post("/api/tts")
async def text_to_speech(req: TtsRequest) -> dict[str, Any]:
    if not req.text.strip():
        raise HTTPException(status_code=400, detail="text is required")

    job_id = _tts_job_id(req)
    speaker = req.speaker or "Emma"
    started = time.monotonic()
    logger.info(
        "vibevoice.tts.start job_id=%s speaker=%s format=%s text_length=%s",
        job_id,
        speaker,
        req.format,
        len(req.text),
    )

    try:
        async with _runtime_lock:
            result = await runtime().text_to_speech(
                text=req.text,
                speaker=speaker,
                output_format=req.format,
                job_id=job_id,
                metadata=req.metadata,
            )
        local_path = result.get("local_path")
        if local_path:
            converted = maybe_convert_audio(Path(local_path), req.format)
            result["local_path"] = str(converted)
    except HTTPException:
        raise
    except Exception as exc:
        logger.exception(
            "vibevoice.tts.failed job_id=%s speaker=%s format=%s text_length=%s elapsed=%.2fs",
            job_id,
            speaker,
            req.format,
            len(req.text),
            time.monotonic() - started,
        )
        raise HTTPException(status_code=500, detail=_error_detail("TTS inference", job_id=job_id, exc=exc)) from exc

    logger.info(
        "vibevoice.tts.success job_id=%s speaker=%s format=%s text_length=%s elapsed=%.2fs",
        job_id,
        speaker,
        req.format,
        len(req.text),
        time.monotonic() - started,
    )
    return {"success": True, **result}


@app.post("/api/transcribe")
async def transcribe(req: TranscribeRequest) -> dict[str, Any]:
    if not req.audio_url.startswith(("https://", "http://")):
        raise HTTPException(status_code=400, detail="audio_url must be http(s)")

    job_id = _transcribe_job_id(req)
    started = time.monotonic()
    logger.info(
        "vibevoice.asr.start job_id=%s audio_url=%s hotwords=%s language=%s",
        job_id,
        req.audio_url,
        len(req.hotwords),
        req.language or "",
    )

    with tempfile.TemporaryDirectory(prefix="videosync-vibevoice-") as tmp:
        audio_path = Path(tmp) / "input_audio"
        async with httpx.AsyncClient(timeout=120.0, follow_redirects=True) as client:
            response = await client.get(req.audio_url)
            response.raise_for_status()
            audio_path.write_bytes(response.content)

        try:
            async with _runtime_lock:
                result = await runtime().transcribe(
                    audio_path=audio_path,
                    hotwords=req.hotwords,
                    language=req.language,
                    context_info=req.context_info,
                    job_id=job_id,
                    metadata=req.metadata,
                )
        except HTTPException:
            raise
        except Exception as exc:
            logger.exception(
                "vibevoice.asr.failed job_id=%s hotwords=%s language=%s elapsed=%.2fs",
                job_id,
                len(req.hotwords),
                req.language or "",
                time.monotonic() - started,
            )
            raise HTTPException(status_code=500, detail=_error_detail("ASR inference", job_id=job_id, exc=exc)) from exc

    logger.info(
        "vibevoice.asr.success job_id=%s hotwords=%s language=%s elapsed=%.2fs",
        job_id,
        len(req.hotwords),
        req.language or "",
        time.monotonic() - started,
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
