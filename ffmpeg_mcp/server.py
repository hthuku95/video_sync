"""
FFmpeg MCP — processes media directly from/to R2 via presigned URLs.
Temp files destroyed after each request. Zero local disk persistence.
"""
import asyncio
import atexit
import os
import signal
import shutil
import tempfile
from contextlib import suppress
from pathlib import Path

import boto3
from fastapi import FastAPI, HTTPException
from pydantic import BaseModel

app = FastAPI(title="FFmpeg MCP")

# ── R2 config ──────────────────────────────────────────────────────────────
R2_ACCOUNT_ID = os.environ.get("R2_ACCOUNT_ID")
R2_ACCESS_KEY_ID = os.environ.get("R2_ACCESS_KEY_ID")
R2_SECRET_ACCESS_KEY = os.environ.get("R2_SECRET_ACCESS_KEY")
R2_BUCKET = os.environ.get("R2_BUCKET", "videosync-production")
R2_ENDPOINT = f"https://{R2_ACCOUNT_ID}.r2.cloudflarestorage.com" if R2_ACCOUNT_ID else None

s3 = None
if R2_ACCOUNT_ID and R2_ACCESS_KEY_ID and R2_SECRET_ACCESS_KEY:
    s3 = boto3.client(
        "s3",
        endpoint_url=R2_ENDPOINT,
        aws_access_key_id=R2_ACCESS_KEY_ID,
        aws_secret_access_key=R2_SECRET_ACCESS_KEY,
        region_name="auto",
    )

# ── Temp directory lifecycle ────────────────────────────────────────────────
ROOT_TEMP = Path("/tmp/ffmpeg_mcp")
ROOT_TEMP.mkdir(parents=True, exist_ok=True)

_cleanup_dirs: set[Path] = set()


def _cleanup_all():
    for d in list(_cleanup_dirs):
        with suppress(Exception):
            shutil.rmtree(str(d))
    _cleanup_dirs.clear()


atexit.register(_cleanup_all)
signal.signal(signal.SIGTERM, lambda *_: _cleanup_all())
signal.signal(signal.SIGINT, lambda *_: _cleanup_all())


def make_temp_dir() -> Path:
    d = Path(tempfile.mkdtemp(dir=str(ROOT_TEMP)))
    _cleanup_dirs.add(d)
    return d


def release_temp_dir(d: Path):
    _cleanup_dirs.discard(d)
    with suppress(Exception):
        shutil.rmtree(str(d))


# ── R2 helpers ──────────────────────────────────────────────────────────────

def presign_url(key: str, expires_in: int = 3600) -> str:
    if not s3:
        raise RuntimeError("R2 not configured")
    return s3.generate_presigned_url(
        "get_object",
        Params={"Bucket": R2_BUCKET, "Key": key},
        ExpiresIn=expires_in,
    )


def upload_file(local_path: str | Path, key: str) -> str:
    if not s3:
        raise RuntimeError("R2 not configured")
    s3.upload_file(str(local_path), R2_BUCKET, key)
    return presign_url(key)


# ── Models ──────────────────────────────────────────────────────────────────

class ProcessRequest(BaseModel):
    input_url: str | None = None
    input_key: str | None = None
    ffmpeg_args: list[str] = []
    output_key: str


class ProcessResponse(BaseModel):
    output_url: str
    output_key: str
    ffmpeg_stderr: str = ""


class HealthResponse(BaseModel):
    status: str
    r2_configured: bool
    ffmpeg_version: str = ""


# ── Endpoints ───────────────────────────────────────────────────────────────

@app.get("/health")
async def health():
    r = await asyncio.create_subprocess_exec(
        "ffmpeg", "-version",
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )
    out, _ = await r.communicate()
    ver = out.decode(errors="replace").split("\n")[0] if r.returncode == 0 else "unknown"
    return HealthResponse(status="ok", r2_configured=s3 is not None, ffmpeg_version=ver)


@app.post("/process", response_model=ProcessResponse)
async def process(req: ProcessRequest):
    if req.input_url and req.input_key:
        raise HTTPException(400, "Provide input_url or input_key, not both")
    if req.input_key:
        input_url = presign_url(req.input_key)
    elif req.input_url:
        input_url = req.input_url
    else:
        raise HTTPException(400, "Must provide input_url or input_key")

    tmp = make_temp_dir()
    try:
        out_name = req.output_key.replace("/", "_")
        output_path = tmp / out_name

        cmd = ["ffmpeg", "-y", "-i", input_url, *req.ffmpeg_args, str(output_path)]

        proc = await asyncio.create_subprocess_exec(
            *cmd,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        _, stderr = await proc.communicate()
        err_text = stderr.decode(errors="replace")

        if proc.returncode != 0:
            raise HTTPException(
                422,
                f"FFmpeg exited {proc.returncode}: {err_text[-2000:]}",
            )

        if not output_path.exists():
            raise HTTPException(422, "FFmpeg completed but no output file produced")

        output_url = upload_file(output_path, req.output_key)
        return ProcessResponse(
            output_url=output_url,
            output_key=req.output_key,
            ffmpeg_stderr=err_text[-2000:],
        )
    finally:
        release_temp_dir(tmp)


if __name__ == "__main__":
    import uvicorn
    uvicorn.run(app, host="0.0.0.0", port=8001)
