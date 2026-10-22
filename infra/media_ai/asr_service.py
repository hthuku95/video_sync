#!/usr/bin/env python3
"""Persistent CPU ASR service for VideoSync.

Topology (single media node):

    nginx :8015  /api/transcribe -> 127.0.0.1:8017 (this service)
                 /api/tts/*      -> 127.0.0.1:8016 (VibeVoice TTS)
                 /health         -> VibeVoice TTS health
                 /health/asr     -> this service health

The model is loaded once at startup. `VIBEVOICE_ASR_CONCURRENCY` bounds
simultaneous inference in this process; `VIBEVOICE_ASR_MAX_QUEUE` bounds
waiting requests. Scale horizontally by adding replicas/nodes behind the
internal ELB (or point the gateway's ASR upstream at an ASR-only ELB).

The service is stateless apart from the model: each request downloads its
audio to a temporary directory and removes it afterwards.
"""

from __future__ import annotations

import asyncio
import hmac
import logging
import os
import re
import tempfile
import time
import uuid
from contextlib import asynccontextmanager
from pathlib import Path
from typing import Any
from urllib.parse import urlparse

import httpx
from fastapi import FastAPI, HTTPException, Request
from fastapi.responses import PlainTextResponse
from pydantic import BaseModel, Field

from faster_whisper import WhisperModel


def _int_env(name: str, default: int) -> int:
    try:
        value = int(os.getenv(name, str(default)))
        return value if value > 0 else default
    except ValueError:
        return default


def _float_env(name: str, default: float) -> float:
    try:
        value = float(os.getenv(name, str(default)))
        return value if value > 0 else default
    except ValueError:
        return default


LOG_LEVEL = os.getenv("LOG_LEVEL", "INFO").upper()
logging.basicConfig(
    level=getattr(logging, LOG_LEVEL, logging.INFO),
    format="%(asctime)s %(levelname)s %(name)s %(message)s",
)
logger = logging.getLogger("videosync.asr")

MODEL_NAME = os.getenv("VIBEVOICE_ASR_MODEL", "base.en")
CPU_THREADS = _int_env("VIBEVOICE_ASR_CPU_THREADS", 2)
COMPUTE_TYPE = os.getenv("VIBEVOICE_ASR_COMPUTE_TYPE", "int8")
BEAM_SIZE = _int_env("VIBEVOICE_ASR_BEAM_SIZE", 5)
MAX_WORDS = _int_env("VIBEVOICE_ASR_MAX_WORDS", 5)
CONCURRENCY = _int_env("VIBEVOICE_ASR_CONCURRENCY", 1)
MAX_QUEUE = _int_env("VIBEVOICE_ASR_MAX_QUEUE", 32)
QUEUE_TIMEOUT = _float_env("VIBEVOICE_ASR_QUEUE_TIMEOUT", 240.0)
MAX_DOWNLOAD_MB = _int_env("VIBEVOICE_ASR_MAX_DOWNLOAD_MB", 200)
USE_VAD = os.getenv("VIBEVOICE_ASR_VAD", "1").lower() not in ("0", "false", "no")
API_KEY = os.getenv("VIBEVOICE_SERVICE_API_KEY", "").strip()

_model: WhisperModel | None = None
_semaphore: asyncio.Semaphore | None = None
_state: dict[str, float] = {
    "requests": 0,
    "failures": 0,
    "queue_timeouts": 0,
    "active": 0,
    "pending": 0,
    "audio_seconds": 0.0,
    "inference_seconds": 0.0,
}
_state_lock = asyncio.Lock()


class TranscribeRequest(BaseModel):
    audio_url: str
    hotwords: list[str] = Field(default_factory=list)
    language: str | None = None
    context_info: str | None = None
    job_id: str | None = None
    metadata: dict[str, Any] = Field(default_factory=dict)


def _load_model() -> WhisperModel:
    global _model
    if _model is None:
        started = time.monotonic()
        _model = WhisperModel(
            MODEL_NAME,
            device="cpu",
            compute_type=COMPUTE_TYPE,
            cpu_threads=CPU_THREADS,
            num_workers=1,
        )
        logger.info(
            "asr.model.loaded model=%s compute_type=%s threads=%d elapsed=%.2fs",
            MODEL_NAME,
            COMPUTE_TYPE,
            CPU_THREADS,
            time.monotonic() - started,
        )
    return _model


def _safe_url(url: str) -> str:
    try:
        parsed = urlparse(url)
        return f"{parsed.scheme}://{parsed.netloc}{parsed.path}"
    except Exception:
        return "<invalid>"


def _transcribe_pass(
    model: WhisperModel,
    path: str,
    *,
    language: str | None,
    hotwords: str | None,
    prompt: str | None,
    vad: bool,
) -> tuple[list[dict[str, Any]], list[str], Any, int]:
    segments_iter, info = model.transcribe(
        path,
        language=language,
        beam_size=BEAM_SIZE,
        best_of=BEAM_SIZE,
        vad_filter=vad,
        vad_parameters={"min_silence_duration_ms": 500} if vad else None,
        word_timestamps=True,
        condition_on_previous_text=False,
        no_speech_threshold=0.6,
        hallucination_silence_threshold=2.0,
        hotwords=hotwords,
        initial_prompt=prompt,
    )

    segments: list[dict[str, Any]] = []
    texts: list[str] = []
    raw_segments = 0

    def add_chunk(words: list[Any]) -> None:
        if not words:
            return
        text = "".join((getattr(word, "word", "") or "") for word in words).strip()
        text = re.sub(r"\s+([,.!?;:])", r"\1", text)
        if not text:
            return
        start = getattr(words[0], "start", None)
        end = getattr(words[-1], "end", None)
        if start is None or end is None or float(end) <= float(start):
            return
        segments.append(
            {
                "id": len(segments),
                "start": round(float(start), 3),
                "end": round(float(end), 3),
                "text": text,
            }
        )
        texts.append(text)

    for segment in segments_iter:
        raw_segments += 1
        # Drop likely non-speech hallucinations (music, silence).
        if float(getattr(segment, "no_speech_prob", 0.0) or 0.0) > 0.75:
            continue
        words = [
            word
            for word in (getattr(segment, "words", None) or [])
            if (getattr(word, "word", "") or "").strip()
        ]
        if words:
            chunk: list[Any] = []
            for word in words:
                chunk.append(word)
                token = (getattr(word, "word", "") or "").strip()
                if len(chunk) >= MAX_WORDS or token.endswith((".", "!", "?", ",", ";", ":")):
                    add_chunk(chunk)
                    chunk = []
            if chunk:
                add_chunk(chunk)
            continue

        # Fallback when word timestamps are unavailable.
        text = (getattr(segment, "text", "") or "").strip()
        start = getattr(segment, "start", None)
        end = getattr(segment, "end", None)
        if text and start is not None and end is not None and float(end) > float(start):
            segments.append(
                {
                    "id": len(segments),
                    "start": round(float(start), 3),
                    "end": round(float(end), 3),
                    "text": text,
                }
            )
            texts.append(text)

    return segments, texts, info, raw_segments


def _transcribe_sync(
    path: str,
    language: str | None,
    hotwords: list[str],
    context_info: str | None,
) -> dict[str, Any]:
    model = _load_model()
    lang = (language or "").strip() or None
    hw = ", ".join(word.strip() for word in hotwords if word and word.strip()) or None
    prompt = (context_info or "").strip() or None

    segments, texts, info, raw_segments = _transcribe_pass(
        model,
        path,
        language=lang,
        hotwords=hw,
        prompt=prompt,
        vad=USE_VAD,
    )

    vad_fallback = False
    if not segments and USE_VAD:
        # Silero VAD can classify singing/music-heavy stream audio as non-speech.
        # Retry without VAD so the caption track is not silently empty.
        segments, texts, info, raw_segments = _transcribe_pass(
            model,
            path,
            language=lang,
            hotwords=hw,
            prompt=prompt,
            vad=False,
        )
        vad_fallback = True
        logger.info(
            "asr.vad_fallback segments=%d raw_segments=%d",
            len(segments),
            raw_segments,
        )

    return {
        "provider": "faster-whisper",
        "model": MODEL_NAME,
        "text": " ".join(texts).strip(),
        "language": getattr(info, "language", None) or lang or "en",
        "language_probability": float(getattr(info, "language_probability", 0.0) or 0.0),
        "duration": float(getattr(info, "duration", 0.0) or 0.0),
        "segments": segments,
        "vad": USE_VAD and not vad_fallback,
        "vad_fallback": vad_fallback,
        "raw_segments": raw_segments,
    }


async def _download_audio(url: str, path: Path) -> None:
    max_bytes = MAX_DOWNLOAD_MB * 1024 * 1024
    total = 0
    try:
        async with httpx.AsyncClient(timeout=120.0, follow_redirects=True) as client:
            async with client.stream("GET", url) as response:
                response.raise_for_status()
                with path.open("wb") as handle:
                    async for chunk in response.aiter_bytes():
                        total += len(chunk)
                        if total > max_bytes:
                            raise HTTPException(
                                status_code=400,
                                detail=f"audio exceeds {MAX_DOWNLOAD_MB} MB",
                            )
                        handle.write(chunk)
    except httpx.HTTPError as exc:
        raise HTTPException(status_code=502, detail=f"audio download failed: {exc}") from exc

    if total == 0:
        raise HTTPException(status_code=400, detail="audio file is empty")


def _check_auth(request: Request) -> None:
    if not API_KEY:
        return
    header = request.headers.get("authorization", "")
    if not hmac.compare_digest(header, f"Bearer {API_KEY}"):
        raise HTTPException(status_code=401, detail="unauthorized")


@asynccontextmanager
async def lifespan(_: FastAPI):
    global _semaphore
    _semaphore = asyncio.Semaphore(CONCURRENCY)
    await asyncio.to_thread(_load_model)
    logger.info(
        "asr.ready model=%s compute_type=%s threads=%d concurrency=%d "
        "max_queue=%d queue_timeout=%.0fs",
        MODEL_NAME,
        COMPUTE_TYPE,
        CPU_THREADS,
        CONCURRENCY,
        MAX_QUEUE,
        QUEUE_TIMEOUT,
    )
    yield


app = FastAPI(title="VideoSync ASR", version="1.0.0", lifespan=lifespan)


@app.get("/health")
async def health() -> dict[str, Any]:
    async with _state_lock:
        active = int(_state["active"])
        pending = int(_state["pending"])
    return {
        "ok": _model is not None,
        "service": "videosync-asr",
        "model": MODEL_NAME,
        "device": "cpu",
        "compute_type": COMPUTE_TYPE,
        "concurrency": CONCURRENCY,
        "active": active,
        "pending": pending,
        "max_queue": MAX_QUEUE,
        "queue_timeout_seconds": QUEUE_TIMEOUT,
    }


@app.get("/metrics", response_class=PlainTextResponse)
async def metrics() -> str:
    async with _state_lock:
        snapshot = {key: float(value) for key, value in _state.items()}
    lines = [
        "# HELP videosync_asr_requests_total Transcription requests accepted.",
        "# TYPE videosync_asr_requests_total counter",
        f"videosync_asr_requests_total {snapshot['requests']:.0f}",
        "# HELP videosync_asr_failures_total Inference failures.",
        "# TYPE videosync_asr_failures_total counter",
        f"videosync_asr_failures_total {snapshot['failures']:.0f}",
        "# HELP videosync_asr_queue_timeouts_total Requests rejected after waiting too long.",
        "# TYPE videosync_asr_queue_timeouts_total counter",
        f"videosync_asr_queue_timeouts_total {snapshot['queue_timeouts']:.0f}",
        "# HELP videosync_asr_inference_seconds_total Wall-clock seconds spent in transcription requests.",
        "# TYPE videosync_asr_inference_seconds_total counter",
        f"videosync_asr_inference_seconds_total {snapshot['inference_seconds']:.3f}",
        "# HELP videosync_asr_audio_seconds_total Audio seconds transcribed.",
        "# TYPE videosync_asr_audio_seconds_total counter",
        f"videosync_asr_audio_seconds_total {snapshot['audio_seconds']:.3f}",
        "# HELP videosync_asr_active In-flight transcriptions.",
        "# TYPE videosync_asr_active gauge",
        f"videosync_asr_active {snapshot['active']:.0f}",
        "# HELP videosync_asr_pending Requests waiting for an inference slot.",
        "# TYPE videosync_asr_pending gauge",
        f"videosync_asr_pending {snapshot['pending']:.0f}",
        "# HELP videosync_asr_queue_capacity Maximum pending requests.",
        "# TYPE videosync_asr_queue_capacity gauge",
        f"videosync_asr_queue_capacity {MAX_QUEUE}",
        "# HELP videosync_asr_concurrency Inference slots in this process.",
        "# TYPE videosync_asr_concurrency gauge",
        f"videosync_asr_concurrency {CONCURRENCY}",
        "# HELP videosync_asr_model_info Loaded ASR model.",
        "# TYPE videosync_asr_model_info gauge",
        f'videosync_asr_model_info{{model="{MODEL_NAME}",compute_type="{COMPUTE_TYPE}"}} 1',
    ]
    return "\n".join(lines) + "\n"


@app.post("/api/transcribe")
async def transcribe(req: TranscribeRequest, request: Request) -> dict[str, Any]:
    _check_auth(request)
    if not req.audio_url.startswith(("http://", "https://")):
        raise HTTPException(status_code=400, detail="audio_url must be http(s)")

    job_id = (req.job_id or "").strip() or str(uuid.uuid4())
    safe_url = _safe_url(req.audio_url)
    started = time.monotonic()

    async with _state_lock:
        if int(_state["pending"]) >= MAX_QUEUE:
            _state["queue_timeouts"] += 1
            raise HTTPException(status_code=429, detail="ASR queue is full")
        _state["pending"] += 1
        _state["requests"] += 1

    acquired = False
    try:
        try:
            assert _semaphore is not None
            await asyncio.wait_for(_semaphore.acquire(), timeout=QUEUE_TIMEOUT)
            acquired = True
        except asyncio.TimeoutError:
            async with _state_lock:
                _state["pending"] -= 1
                _state["queue_timeouts"] += 1
            logger.warning("asr.queue.timeout job_id=%s url=%s", job_id, safe_url)
            raise HTTPException(status_code=503, detail="ASR queue wait timed out")
        except BaseException:
            async with _state_lock:
                _state["pending"] -= 1
            raise

        async with _state_lock:
            _state["pending"] -= 1
            _state["active"] += 1

        logger.info(
            "asr.start job_id=%s url=%s language=%s hotwords=%d",
            job_id,
            safe_url,
            req.language or "",
            len(req.hotwords),
        )
        with tempfile.TemporaryDirectory(prefix="videosync-asr-") as tmp:
            audio_path = Path(tmp) / "input_audio"
            await _download_audio(req.audio_url, audio_path)
            payload = await asyncio.to_thread(
                _transcribe_sync,
                str(audio_path),
                req.language,
                req.hotwords,
                req.context_info,
            )

        elapsed = time.monotonic() - started
        async with _state_lock:
            _state["audio_seconds"] += float(payload.get("duration") or 0.0)
            _state["inference_seconds"] += elapsed
        logger.info(
            "asr.success job_id=%s segments=%d language=%s elapsed=%.2fs",
            job_id,
            len(payload.get("segments") or []),
            payload.get("language"),
            elapsed,
        )
        return {
            "success": True,
            **payload,
            "job_id": job_id,
            "metadata": req.metadata,
        }
    except HTTPException:
        raise
    except Exception as exc:
        elapsed = time.monotonic() - started
        async with _state_lock:
            _state["failures"] += 1
        logger.exception(
            "asr.failed job_id=%s url=%s elapsed=%.2fs",
            job_id,
            safe_url,
            elapsed,
        )
        raise HTTPException(
            status_code=500,
            detail=f"ASR inference failed: {exc}",
        ) from exc
    finally:
        if acquired:
            assert _semaphore is not None
            _semaphore.release()
            async with _state_lock:
                _state["active"] -= 1
