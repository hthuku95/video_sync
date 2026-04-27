from __future__ import annotations

import argparse
import copy
import os
from pathlib import Path


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="VideoSync lower-memory adapter for vendored VibeVoice TTS")
    parser.add_argument("--repo_dir", required=True)
    parser.add_argument("--model_path", required=True)
    parser.add_argument("--txt_path", required=True)
    parser.add_argument("--speaker_name", required=True)
    parser.add_argument("--output_dir", required=True)
    parser.add_argument("--device", default="cpu")
    parser.add_argument("--cfg_scale", type=float, default=1.5)
    return parser.parse_args()


def resolve_cpu_dtype(torch_module, raw_value: str):
    value = (raw_value or "bfloat16").strip().lower()
    if value == "float16":
        return torch_module.float16
    if value == "float32":
        return torch_module.float32
    return torch_module.bfloat16


def load_modules(repo_dir: Path):
    import sys

    sys.path.insert(0, str(repo_dir))

    import torch
    from vibevoice.modular.modeling_vibevoice_streaming_inference import (
        VibeVoiceStreamingForConditionalGenerationInference,
    )
    from vibevoice.processor.vibevoice_streaming_processor import VibeVoiceStreamingProcessor

    return torch, VibeVoiceStreamingForConditionalGenerationInference, VibeVoiceStreamingProcessor


def find_voice_sample(repo_dir: Path, speaker_name: str) -> Path:
    voices_dir = repo_dir / "demo" / "voices" / "streaming_model"
    speaker_key = speaker_name.lower()

    exact = voices_dir / f"{speaker_key}.pt"
    if exact.exists():
        return exact

    matches = sorted(voices_dir.glob(f"*{speaker_key}*.pt"))
    if matches:
        return matches[0]

    default_voice = next(iter(sorted(voices_dir.glob("*.pt"))), None)
    if default_voice is None:
        raise FileNotFoundError(f"No VibeVoice streaming voice presets found in {voices_dir}")
    return default_voice


def main() -> None:
    args = parse_args()
    repo_dir = Path(args.repo_dir).resolve()
    output_dir = Path(args.output_dir).resolve()
    txt_path = Path(args.txt_path).resolve()

    if not txt_path.exists():
        raise FileNotFoundError(f"Input text file not found: {txt_path}")

    torch, inference_cls, processor_cls = load_modules(repo_dir)
    script_text = txt_path.read_text(encoding="utf-8").strip()
    if not script_text:
        raise ValueError("Input text file was empty")

    if args.device != "cpu":
        raise ValueError("VideoSync TTS adapter currently supports cpu-only inference")

    torch_dtype = resolve_cpu_dtype(torch, os.getenv("VIBEVOICE_TTS_CPU_DTYPE", "bfloat16"))
    processor = processor_cls.from_pretrained(args.model_path)
    model = inference_cls.from_pretrained(
        args.model_path,
        torch_dtype=torch_dtype,
        device_map="cpu",
        attn_implementation="sdpa",
        low_cpu_mem_usage=True,
    )
    model.eval()
    model.set_ddpm_inference_steps(num_steps=5)

    voice_sample = find_voice_sample(repo_dir, args.speaker_name)
    all_prefilled_outputs = torch.load(voice_sample, map_location="cpu", weights_only=False)

    inputs = processor.process_input_with_cached_prompt(
        text=script_text,
        cached_prompt=all_prefilled_outputs,
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
        cfg_scale=args.cfg_scale,
        tokenizer=processor.tokenizer,
        generation_config={"do_sample": False},
        verbose=False,
        all_prefilled_outputs=copy.deepcopy(all_prefilled_outputs),
    )

    output_dir.mkdir(parents=True, exist_ok=True)
    output_path = output_dir / f"{txt_path.stem}_generated.wav"
    processor.save_audio(outputs.speech_outputs[0], output_path=str(output_path))
    print(output_path)


if __name__ == "__main__":
    main()
