from __future__ import annotations

import asyncio
import os
import shlex
from dataclasses import dataclass
from pathlib import Path
from typing import Any
from uuid import uuid4


@dataclass
class VibeVoiceRuntime:
    tts_model: str
    asr_model: str
    output_dir: Path

    @classmethod
    def from_env(cls) -> "VibeVoiceRuntime":
        return cls(
            tts_model=os.getenv("VIBEVOICE_TTS_MODEL", "microsoft/VibeVoice-Realtime-0.5B"),
            asr_model=os.getenv("VIBEVOICE_ASR_MODEL", "microsoft/VibeVoice-ASR-7B"),
            output_dir=Path(os.getenv("VIBEVOICE_OUTPUT_DIR", "/tmp/videosync-vibevoice")),
        )

    @staticmethod
    def _format_command(command: str, **values: str) -> list[str]:
        return [part.format(**values) for part in shlex.split(command)]

    async def text_to_speech(
        self,
        *,
        text: str,
        speaker: str,
        output_format: str,
        job_id: str,
        metadata: dict[str, Any],
    ) -> dict[str, Any]:
        self.output_dir.mkdir(parents=True, exist_ok=True)
        output_path = self.output_dir / f"{job_id or uuid4()}.{output_format}"

        # The official VibeVoice package/runtime is intentionally not vendored
        # in this repo. Keep this method as the adapter seam: install the model
        # package in the service image, then replace this command with the
        # supported inference call for the chosen VibeVoice model.
        command = os.getenv("VIBEVOICE_TTS_COMMAND")
        if not command:
            raise RuntimeError(
                "VIBEVOICE_TTS_COMMAND is not configured. Install VibeVoice and set "
                "a command that writes audio to the provided output path."
            )

        args = self._format_command(
            command,
            model=self.tts_model,
            speaker=speaker,
            output=str(output_path),
            text=text,
        )
        proc = await asyncio.create_subprocess_exec(
            *args,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        stdout, stderr = await proc.communicate()
        if proc.returncode != 0:
            raise RuntimeError(stderr.decode("utf-8", errors="replace") or "VibeVoice TTS failed")

        return {
            "provider": "vibevoice",
            "model": self.tts_model,
            "speaker": speaker,
            "local_path": str(output_path),
            "stdout": stdout.decode("utf-8", errors="replace"),
            "metadata": metadata,
        }

    async def transcribe(
        self,
        *,
        audio_path: Path,
        hotwords: list[str],
        language: str | None,
        job_id: str,
        metadata: dict[str, Any],
    ) -> dict[str, Any]:
        command = os.getenv("VIBEVOICE_ASR_COMMAND")
        if not command:
            raise RuntimeError(
                "VIBEVOICE_ASR_COMMAND is not configured. Install VibeVoice-ASR and set "
                "a command that returns structured transcription JSON or text."
            )

        args = self._format_command(
            command,
            model=self.asr_model,
            input=str(audio_path),
            hotwords=",".join(hotwords),
            language=language or "",
            job_id=job_id,
        )
        proc = await asyncio.create_subprocess_exec(
            *args,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        stdout, stderr = await proc.communicate()
        if proc.returncode != 0:
            raise RuntimeError(stderr.decode("utf-8", errors="replace") or "VibeVoice ASR failed")

        text = stdout.decode("utf-8", errors="replace").strip()
        return {
            "provider": "vibevoice-asr",
            "model": self.asr_model,
            "text": text,
            "segments": [],
            "metadata": metadata,
        }
