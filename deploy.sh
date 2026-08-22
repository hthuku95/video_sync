#!/bin/bash
# Build-host deploy script for video-editor.
# Run ON build-host-eu (13.60.35.61):  cd /home/ubuntu/video_editor && bash deploy.sh
#
# Guards against the Aug 22 2026 incident where docker shipped a STALE prebuilt
# binary (single-stage Dockerfile COPY + legacy-builder layer cache). The script
# refuses to push unless the binary inside the image byte-matches the fresh
# cargo build output.

set -euo pipefail
cd /home/ubuntu/video_editor

echo "==> [1/5] git pull"
git pull --ff-only origin master

echo "==> [2/5] cargo build --release (~15-25 min on t3.large)"
/home/ubuntu/.cargo/bin/cargo build --release
test -x target/release/video_editor || { echo "FATAL: cargo produced no binary"; exit 1; }

HOST_MD5=$(md5sum target/release/video_editor | cut -d' ' -f1)
echo "    host binary md5: $HOST_MD5"

echo "==> [3/5] docker build (--no-cache)"
docker build --no-cache -t video-editor:latest -f Dockerfile .

CID=$(docker create video-editor:latest)
IMG_MD5=$(docker export "$CID" | tar -xOf - usr/local/bin/video_editor | md5sum | cut -d' ' -f1)
docker rm -f "$CID" > /dev/null
echo "    image binary md5: $IMG_MD5"

if [ "$IMG_MD5" != "$HOST_MD5" ]; then
  echo "FATAL: binary inside image does NOT match fresh build — refusing to push."
  exit 1
fi
echo "    OK: image contains the fresh binary"

echo "==> [4/5] ECR login + tag + push"
aws ecr get-login-password --region eu-north-1 \
  | docker login --username AWS --password-stdin 960066381428.dkr.ecr.eu-north-1.amazonaws.com > /dev/null
docker tag video-editor:latest 960066381428.dkr.ecr.eu-north-1.amazonaws.com/video-editor:latest
docker push 960066381428.dkr.ecr.eu-north-1.amazonaws.com/video-editor:latest

echo "==> [5/5] DONE"
echo "Next (from local machine): aws ecs update-service --cluster video-editor-fargate \
--service video-editor-api --task-definition video-editor-fargate:{rev} --force-new-deployment"
