# Media AI services (ASR + TTS)

Live topology on the media node (`34.192.36.59`, us-east-1, t3.large, 2 vCPU / 8 GB).
This replaces the old single VibeVoice process that handled ASR and TTS in-process.

```
internal ALB video-editor-media :8015
  -> target group vibevoice-8015 -> media node nginx :8015
       /api/transcribe  -> 127.0.0.1:8017  vibevoice-asr.service  (faster-whisper)
       /api/tts/*       -> 127.0.0.1:8016  vibevoice-tts.service  (VibeVoice)
       /health          -> 8016
       /health/asr      -> 8017
       /metrics/asr     -> 8017 Prometheus text
```

The Rust API keeps the same `VIBEVOICE_SERVICE_URL` and sends
`Authorization: Bearer $VIBEVOICE_SERVICE_API_KEY`. Both services bind localhost;
only nginx listens on `0.0.0.0:8015`.

## Files

| Path | Purpose |
|------|---------|
| `/opt/media/ai-asr/asr_service.py` | faster-whisper ASR service |
| `/opt/media/ai-tts/tts_service.py` | persistent VibeVoice TTS service |
| `/etc/nginx/conf.d/vibevoice-gateway.conf` | gateway path split |
| `/etc/systemd/system/vibevoice-asr.service` | ASR unit |
| `/etc/systemd/system/vibevoice-tts.service` | TTS unit |
| `/opt/media/media.env` | shared env + `VIBEVOICE_SERVICE_API_KEY` |

Repo copies live in `infra/media_ai/`. The media node is patched, never
`git pull`ed.

## Models and settings

ASR:

- `faster-whisper 1.2.1`, model `base.en`, CPU `int8`, 2 threads.
- Model loaded once; queue 32, concurrency 1, queue timeout 240 s.
- `VIBEVOICE_ASR_VAD=1` by default. If the VAD pass returns 0 segments, the
  service retries once with `vad_filter=False` (singing/music-heavy stream
  audio). Segments with `no_speech_prob > 0.75` are dropped.
- Response includes `segments[]`, `vad`, `vad_fallback`, `raw_segments`.
- Measured: 2.5 s for 15 s audio; 26 s for a 45 s VAD-fallback clip.

TTS:

- `microsoft/VibeVoice-Realtime-0.5B`, CPU `bfloat16`, 2 threads.
- Queue 16, concurrency 1; model/processor/voice cached.
- `torch.inference_mode()` is mandatory. Without it peak RSS was 6.24 GB;
  with it ~3.3 GB.
- `VIBEVOICE_TTS_CPU_DTYPE=bfloat16`; do not switch to `float32` on this node.
- Measured: 21 s cold / 9.5 s warm for a 25-char phrase.

## Operations

```bash
sudo systemctl status vibevoice-tts vibevoice-asr nginx
sudo journalctl -u vibevoice-asr -f
sudo journalctl -u vibevoice-tts -f

curl -fsS http://127.0.0.1:8015/health
curl -fsS http://127.0.0.1:8015/health/asr
curl -fsS http://127.0.0.1:8015/metrics/asr

curl -fsS -X POST http://127.0.0.1:8015/api/transcribe \
  -H "Authorization: Bearer $VIBEVOICE_SERVICE_API_KEY" \
  -H 'Content-Type: application/json' \
  -d '{"audio_url":"https://example.com/clip.mp4","language":"en"}'
```

Deploy a change by copying the file over the deployed path and restarting only
the affected unit:

```bash
sudo install -m 644 asr_service.py /opt/media/ai-asr/asr_service.py
sudo systemctl restart vibevoice-asr
```

## Scaling

- One model per process; keep `concurrency=1` on 2 vCPU. Scaling means more
  processes/nodes, not more threads.
- TTS: add media nodes to the `vibevoice-8015` target group, or put an ALB in
  front of dedicated TTS nodes. The gateway and services are stateless.
- ASR: point the nginx `/api/transcribe` upstream at an ASR-only target group
  or an `upstream` block with multiple nodes. Do not route ASR through the TTS
  process again.
- Memory budget on one t3.large: TTS ~3.3 GB peak + ASR ~0.42 GB + OS. Concurrent
  TTS+ASR was verified with no OOM; do not add a third large model to this node.
- Long education narration is CPU-bound. Split scripts into scene-level TTS
  calls, or add TTS replicas. Do not raise model concurrency on this hardware.

## Rollback

```bash
sudo systemctl stop vibevoice-tts vibevoice-asr
sudo rm /etc/nginx/conf.d/vibevoice-gateway.conf
sudo nginx -t && sudo nginx -s reload
sudo systemctl enable --now vibevoice   # old service on :8016
```

The old `vibevoice.service` is disabled but its patched adapter remains.
`VIBEVOICE_SERVICE_URL` can stay pointed at the ALB `:8015` only while nginx is
running; if you remove the gateway, point the app at the old service port.
