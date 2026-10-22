#!/usr/bin/env python3
"""Persistent CPU VibeVoice TTS service for VideoSync.

Topology (single media node):

    nginx :8015  /api/tts/*      -> 127.0.0.1:8016 (this service)
                 /api/transcribe -> 127.0.0.1:8017 (faster-whisper ASR)
                 /health         -> this service
                 /health/asr     -> ASR service

The VibeVoice processor/model are loaded once at startup and reused. Voice
prompts are cached by file path. `VIBEVOICE_TTS_CONCURRENCY` bounds concurrent
generations in this process; `VIBEVOICE_TTS_MAX_QUEUE` bounds waiting requests.
Scale horizontally by adding replicas/nodes. Each replica holds one model
(~1.5 GB idle, ~3.1 GB peak with inference mode).
"""

from __future__ import annotations

import asyncio
import base64
import copy
import hmac
import logging
import os
import subprocess
import sys
import tempfile
import time
import uuid
from contextlib import asynccontextmanager
from pathlib import Path
from typing import Any

import torch
from fastapi import FastAPI, HTTPException, Request
from fastapi.responses import PlainTextResponse
from pydantic import BaseModel, Field

REPO_DIR = Path(os.getenv("VIBEVOICE_REPO_DIR", "/opt/media/vibevoice-runtime")).resolve()
if str(REPO_DIR) not in sys.path:
    sys.path.insert(0, str(REPO_DIR))

from vibevoice.modular.modeling_vibevoice_streaming_inference import (  # noqa: E402
    VibeVoiceStreamingForConditionalGenerationInference,
)
from vibevoice.processor.vibevoice_streaming_processor import (  # noqa: E402
    VibeVoiceStreamingProcessor,
)


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
logger = logging.getLogger("videosync.tts")

MODEL_NAME = os.getenv("VIBEVOICE_TTS_MODEL", "microsoft/VibeVoice-Realtime-0.5B")
CPU_DTYPE = os.getenv("VIBEVOICE_TTS_CPU_DTYPE", "bfloat16").strip().lower()
THREADS = _int_env("VIBEVOICE_TTS_THREADS", 2)
CFG_SCALE = _float_env("VIBEVOICE_TTS_CFG_SCALE", 1.5)
DDPM_STEPS = _int_env("VIBEVOICE_TTS_DDPM_STEPS", 5)
CONCURRENCY = _int_env("VIBEVOICE_TTS_CONCURRENCY", 1)
MAX_QUEUE = _int_env("VIBEVOICE_TTS_MAX_QUEUE", 16)
QUEUE_TIMEOUT = _float_env("VIBEVOICE_TTS_QUEUE_TIMEOUT", 240.0)
DEFAULT_SPEAKER = os.getenv("VIBEVOICE_TTS_DEFAULT_SPEAKER", "Emma")
FFMPEG_BIN = os.getenv("FFMPEG_BIN", "ffmpeg")
API_KEY = os.getenv("VIBEVOICE_SERVICE_API_KEY", "").strip()
VOICES_DIR = REPO_DIR / "demo" / "voices" / "streaming_model"

_model: Any = None
_processor: Any = None
_voice_paths: dict[str, Path] = {}
_voices: dict[Path, Any] = {}
_semaphore: asyncio.Semaphore | None = None
_state: dict[str, float] = {
    "requests": 0,
    "failures": 0,
    "queue_timeouts": 0,
    "active": 0,
    "pending": 0,
    "synthesis_seconds": 0.0,
    "audio_bytes": 0.0,
}
_state_lock = asyncio.Lock()


class TtsRequest(BaseModel):
    text: str = Field(..., min_length=1)
    speaker: str | None = DEFAULT_SPEAKER
    format: str = "wav"
    job_id: str | None = None
    metadata: dict[str, Any] = Field(default_factory=dict)


def _torch_dtype() -> torch.dtype:
    if CPU_DTYPE == "float32":
        return torch.float32
    # bfloat16 is the supported low-memory CPU path; float16 is not usable on CPU.
    return torch.bfloat16


def _load_model() -> tuple[Any, Any]:
    global _model, _processor
    if _model is None:
        started = time.monotonic()
        torch.set_num_threads(THREADS)
        _processor = VibeVoiceStreamingProcessor.from_pretrained(MODEL_NAME)
        _model = VibeVoiceStreamingForConditionalGenerationInference.from_pretrained(
            MODEL_NAME,
            dtype=_torch_dtype(),
            device_map="cpu",
            attn_implementation="sdpa",
            low_cpu_mem_usage=True,
        )
        _model.eval()
        _model.set_ddpm_inference_steps(num_steps=DDPM_STEPS)
        logger.info(
            "tts.model.loaded model=%s dtype=%s threads=%d elapsed=%.2fs",
            MODEL_NAME,
            CPU_DTYPE,
            THREADS,
            time.monotonic() - started,
        )
    return _processor, _model


def _resolve_voice(speaker: str | None) -> Path:
    name = (speaker or DEFAULT_SPEAKER).strip() or DEFAULT_SPEAKER
    cached = _voice_paths.get(name)
    if cached is not None:
        return cached

    key = name.lower()
    exact = VOICES_DIR / f"{key}.pt"
    if exact.exists():
        path = exact
    else:
        matches = sorted(VOICES_DIR.glob(f"*{key}*.pt"))
        if matches:
            path = matches[0]
        else:
            path = next(iter(sorted(VOICES_DIR.glob("*.pt"))), None)
    if path is None:
        raise FileNotFoundError(f"No VibeVoice voice presets found in {VOICES_DIR}")
    _voice_paths[name] = path
    return path


def _load_voice(speaker: str | None) -> Any:
    path = _resolve_voice(speaker)
    cached = _voices.get(path)
    if cached is None:
        cached = torch.load(path, map_location="cpu", weights_only=False)
        _voices[path] = cached
    return cached


def _synthesize_sync(text: str, speaker: str | None, output_format: str) -> tuple[bytes, str]:
    processor, model = _load_model()
    voice = _load_voice(speaker)
    requested_format = (output_format or "wav").strip().lower().lstrip(".") or "wav"

    with tempfile.TemporaryDirectory(prefix="videosync-tts-") as tmp:
        wav_path = Path(tmp) / "output.wav"
        with torch.inference_mode():
            cached_prompt = copy.deepcopy(voice)
            inputs = processor.process_input_with_cached_prompt(
                text=text,
                cached_prompt=cached_prompt,
                padding=True,
                return_tensors="pt",
                return_attention_mask=True,
            )
            for key, value in list(inputs.items()):
                if torch.is_tensor(value):
                    inputs[key] = value.to("cpu")

            outputs = model.generate(
                **inputs,
                max_new_tokens=None,
                cfg_scale=CFG_SCALE,
                tokenizer=processor.tokenizer,
                generation_config={"do_sample": False},
                verbose=False,
                all_prefilled_outputs=copy.deepcopy(cached_prompt),
            )
            processor.save_audio(outputs.speech_outputs[0], output_path=str(wav_path))

        result_path = wav_path
        if requested_format != "wav":
            converted = Path(tmp) / f"output.{requested_format}"
            completed = subprocess.run(
                [FFMPEG_BIN, "-y", "-i", str(wav_path), str(converted)],
                capture_output=True,
            )
            if completed.returncode != 0:
                detail = completed.stderr.decode("utf-8", errors="replace")[-800:]
                raise RuntimeError(f"ffmpeg conversion to {requested_format} failed: {detail}")
            result_path = converted

        return result_path.read_bytes(), requested_format


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
    await asyncio.to_thread(_load_voice, DEFAULT_SPEAKER)
    logger.info(
        "tts.ready model=%s dtype=%s threads=%d concurrency=%d max_queue=%d queue_timeout=%.0fs voices=%d",
        MODEL_NAME,
        CPU_DTYPE,
        THREADS,
        CONCURRENCY,
        MAX_QUEUE,
        QUEUE_TIMEOUT,
        len(list(VOICES_DIR.glob("*.pt"))),
    )
    yield


app = FastAPI(title="VideoSync VibeVoice TTS", version="1.0.0", lifespan=lifespan)


@app.get("/health")
async def health() -> dict[str, Any]:
    async with _state_lock:
        active = int(_state["active"])
        pending = int(_state["pending"])
    return {
        "ok": _model is not None,
        "service": "videosync-tts",
        "model": MODEL_NAME,
        "device": "cpu",
        "dtype": CPU_DTYPE,
        "speaker_count": len(list(VOICES_DIR.glob("*.pt"))),
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
        "# HELP videosync_tts_requests_total TTS requests accepted.",
        "# TYPE videosync_tts_requests_total counter",
        f"videosync_tts_requests_total {snapshot['requests']:.0f}",
        "# HELP videosync_tts_failures_total Synthesis failures.",
        "# TYPE videosync_tts_failures_total counter",
        f"videosync_tts_failures_total {snapshot['failures']:.0f}",
        "# HELP videosync_tts_queue_timeouts_total Requests rejected after waiting too long.",
        "# TYPE videosync_tts_queue_timeouts_total counter",
        f"videosync_tts_queue_timeouts_total {snapshot['queue_timeouts']:.0f}",
        "# HELP videosync_tts_synthesis_seconds_total Wall-clock seconds spent in TTS requests.",
        "# TYPE videosync_tts_synthesis_seconds_total counter",
        f"videosync_tts_synthesis_seconds_total {snapshot['synthesis_seconds']:.3f}",
        "# HELP videosync_tts_audio_bytes_total Encoded audio bytes returned.",
        "# TYPE videosync_tts_audio_bytes_total counter",
        f"videosync_tts_audio_bytes_total {snapshot['audio_bytes']:.0f}",
        "# HELP videosync_tts_active In-flight syntheses.",
        "# TYPE videosync_tts_active gauge",
        f"videosync_tts_active {snapshot['active']:.0f}",
        "# HELP videosync_tts_pending Requests waiting for a synthesis slot.",
        "# TYPE videosync_tts_pending gauge",
        f"videosync_tts_pending {snapshot['pending']:.0f}",
        "# HELP videosync_tts_queue_capacity Maximum pending requests.",
        "# TYPE videosync_tts_queue_capacity gauge",
        f"videosync_tts_queue_capacity {MAX_QUEUE}",
        "# HELP videosync_tts_concurrency Synthesis slots in this process.",
        "# TYPE videosync_tts_concurrency gauge",
        f"videosync_tts_concurrency {CONCURRENCY}",
        "# HELP videosync_tts_model_info Loaded TTS model.",
        "# TYPE videosync_tts_model_info gauge",
        f'videosync_tts_model_info{{model="{MODEL_NAME}",dtype="{CPU_DTYPE}"}} 1',
    ]
    return "\n".join(lines) + "\n"


@app.post("/api/tts/base64")
async def tts_base64(req: TtsRequest, request: Request) -> dict[str, Any]:
    _check_auth(request)
    text = req.text.strip()
    if not text:
        raise HTTPException(status_code=400, detail="text must not be empty")

    speaker = req.speaker or DEFAULT_SPEAKER
    output_format = (req.format or "wav").strip().lower() or "wav"
    job_id = (req.job_id or "").strip() or str(uuid.uuid4())
    started = time.monotonic()

    async with _state_lock:
        if int(_state["pending"]) >= MAX_QUEUE:
            _state["queue_timeouts"] += 1
            raise HTTPException(status_code=429, detail="TTS queue is full")
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
            logger.warning("tts.queue.timeout job_id=%s", job_id)
            raise HTTPException(status_code=503, detail="TTS queue wait timed out")
        except BaseException:
            async with _state_lock:
                _state["pending"] -= 1
            raise

        async with _state_lock:
            _state["pending"] -= 1
            _state["active"] += 1

        logger.info(
            "tts.start job_id=%s speaker=%s format=%s text_length=%d",
            job_id,
            speaker,
            output_format,
            len(text),
        )
        data, actual_format = await asyncio.to_thread(
            _synthesize_sync,
            text,
            speaker,
            output_format,
        )
        elapsed = time.monotonic() - started
        async with _state_lock:
            _state["synthesis_seconds"] += elapsed
            _state["audio_bytes"] += float(len(data))
        logger.info(
            "tts.success job_id=%s bytes=%d format=%s elapsed=%.2fs",
            job_id,
            len(data),
            actual_format,
            elapsed,
        )
        return {
            "success": True,
            "provider": "vibevoice",
            "model": MODEL_NAME,
            "format": actual_format,
            "audio_base64": base64.b64encode(data).decode("ascii"),
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
            "tts.failed job_id=%s speaker=%s format=%s elapsed=%.2fs",
            job_id,
            speaker,
            output_format,
            elapsed,
        )
        raise HTTPException(status_code=500, detail=f"TTS inference failed: {exc}") from exc
    finally:
        if acquired:
            assert _semaphore is not None
            _semaphore.release()
            async with _state_lock:
                _state["active"] -= 1
