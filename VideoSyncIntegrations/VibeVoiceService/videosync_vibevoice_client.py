from __future__ import annotations

import asyncio
import json
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

    def _repo_dir(self) -> Path:
        configured = os.getenv("VIBEVOICE_REPO_DIR")
        if configured:
            return Path(configured)
        return Path(__file__).resolve().parents[1] / "VibeVoice"

    def _streaming_voices_dir(self) -> Path:
        return self._repo_dir() / "demo" / "voices" / "streaming_model"

    def _python_bin(self) -> str:
        return os.getenv("VIBEVOICE_PYTHON_BIN", "python3")

    @staticmethod
    def _format_command(command: str, **values: str) -> list[str]:
        return [part.format(**values) for part in shlex.split(command)]

    def _default_tts_command(
        self,
        *,
        text_path: Path,
        speaker: str,
        output_dir: Path,
    ) -> tuple[list[str], Path]:
        repo_dir = self._repo_dir()
        script_path = Path(__file__).resolve().with_name("vibevoice_tts_adapter.py")
        if not script_path.exists():
            raise RuntimeError(
                f"Unable to locate VideoSync TTS adapter at {script_path}. "
                "Configure VIBEVOICE_TTS_COMMAND explicitly."
            )

        expected_output = output_dir / f"{text_path.stem}_generated.wav"
        command = [
            self._python_bin(),
            str(script_path),
            "--repo_dir",
            str(repo_dir),
            "--model_path",
            self.tts_model,
            "--txt_path",
            str(text_path),
            "--speaker_name",
            speaker,
            "--output_dir",
            str(output_dir),
        ]
        return command, expected_output

    def _default_asr_command(
        self,
        *,
        audio_path: Path,
        hotwords: list[str],
        language: str | None,
        context_info: str | None,
    ) -> list[str]:
        repo_dir = self._repo_dir()
        adapter_path = Path(__file__).resolve().with_name("vibevoice_asr_adapter.py")
        if not adapter_path.exists():
            raise RuntimeError(
                f"Unable to locate VideoSync ASR adapter at {adapter_path}. "
                "Configure VIBEVOICE_ASR_COMMAND explicitly."
            )

        command = [
            self._python_bin(),
            str(adapter_path),
            "--repo_dir",
            str(repo_dir),
            "--model_path",
            self.asr_model,
            "--audio_file",
            str(audio_path),
            "--device",
            os.getenv("VIBEVOICE_ASR_DEVICE", "auto"),
        ]
        if hotwords:
            command.extend(["--hotwords", ",".join(hotwords)])
        if language:
            command.extend(["--language", language])
        if context_info:
            command.extend(["--context_info", context_info])
        return command

    def list_streaming_speakers(self) -> list[dict[str, Any]]:
        voices_dir = self._streaming_voices_dir()
        if not voices_dir.exists():
            return []

        speakers: list[dict[str, Any]] = []
        for voice_file in sorted(voices_dir.glob("*.pt")):
            stem = voice_file.stem
            parts = stem.split("-")
            language = parts[0] if len(parts) > 1 else "en"
            speaker_code = parts[1] if len(parts) > 1 else parts[0]
            style = parts[2] if len(parts) > 2 else None
            speakers.append(
                {
                    "id": stem,
                    "speaker_name": speaker_code,
                    "language": language,
                    "style": style,
                    "experimental": language != "en" or speaker_code.startswith("Spk"),
                    "file": str(voice_file),
                }
            )
        return speakers

    def capabilities(self) -> dict[str, Any]:
        speakers = self.list_streaming_speakers()
        languages = sorted({speaker["language"] for speaker in speakers})
        return {
            "tts": {
                "realtime_model": self.tts_model,
                "single_speaker_only": True,
                "streaming_text_input_upstream": True,
                "default_service_mode": "file_to_audio",
                "available_speakers": speakers,
                "languages": languages,
            },
            "asr": {
                "model": self.asr_model,
                "supports_long_form": True,
                "supports_diarization": True,
                "supports_timestamps": True,
                "supports_hotwords": True,
                "supports_context_info": True,
                "supports_multilingual": True,
            },
        }

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
        request_id = job_id or str(uuid4())
        output_dir = self.output_dir / request_id
        output_dir.mkdir(parents=True, exist_ok=True)
        text_path = output_dir / "input.txt"
        text_path.write_text(text, encoding="utf-8")

        command = os.getenv("VIBEVOICE_TTS_COMMAND")
        if command:
            output_path = output_dir / f"{request_id}.{output_format}"
            args = self._format_command(
                command,
                model=self.tts_model,
                speaker=speaker,
                output=str(output_path),
                text=text,
                text_path=str(text_path),
                output_dir=str(output_dir),
            )
        else:
            args, output_path = self._default_tts_command(
                text_path=text_path,
                speaker=speaker,
                output_dir=output_dir,
            )

        proc = await asyncio.create_subprocess_exec(
            *args,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        stdout, stderr = await proc.communicate()
        if proc.returncode != 0:
            raise RuntimeError(stderr.decode("utf-8", errors="replace") or "VibeVoice TTS failed")
        if not output_path.exists():
            raise RuntimeError(f"VibeVoice TTS completed but output file was not found at {output_path}")

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
        context_info: str | None,
        job_id: str,
        metadata: dict[str, Any],
    ) -> dict[str, Any]:
        self.output_dir.mkdir(parents=True, exist_ok=True)
        request_id = job_id or str(uuid4())
        output_dir = self.output_dir / request_id
        output_dir.mkdir(parents=True, exist_ok=True)

        command = os.getenv("VIBEVOICE_ASR_COMMAND")
        if not command:
            args = self._default_asr_command(
                audio_path=audio_path,
                hotwords=hotwords,
                language=language,
                context_info=context_info,
            )
            output_path = None
        else:
            output_path = output_dir / f"{request_id}_transcription.json"
            args = self._format_command(
                command,
                model=self.asr_model,
                input=str(audio_path),
                hotwords=",".join(hotwords),
                language=language or "",
                context_info=context_info or "",
                job_id=request_id,
                output=str(output_path),
                output_json=str(output_path),
                output_dir=str(output_dir),
            )
        proc = await asyncio.create_subprocess_exec(
            *args,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        stdout, stderr = await proc.communicate()
        if proc.returncode != 0:
            raise RuntimeError(stderr.decode("utf-8", errors="replace") or "VibeVoice ASR failed")

        if output_path and output_path.exists():
            payload = json.loads(output_path.read_text(encoding="utf-8"))
        else:
            stdout_text = stdout.decode("utf-8", errors="replace").strip()
            try:
                payload = json.loads(stdout_text)
            except json.JSONDecodeError:
                payload = {
                    "provider": "vibevoice-asr",
                    "model": self.asr_model,
                    "text": stdout_text,
                    "segments": [],
                }

        payload["metadata"] = metadata
        return payload
