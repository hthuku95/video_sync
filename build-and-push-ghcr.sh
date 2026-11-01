#!/bin/bash
# Build and push Docker image to GitHub Container Registry (ghcr.io)
# Run this in a GitHub Codespace after creating it from htostudiosbusiness-beep/video_editor

set -e

IMAGE="ghcr.io/htostudiosbusiness-beep/video_editor:latest"
REGISTRY="ghcr.io"

echo "=== Logging in to ghcr.io ==="
echo "$GITHUB_TOKEN" | docker login $REGISTRY -u htostudiosbusiness-beep --password-stdin

echo "=== Building Docker image ==="
docker build -f Dockerfile -t $IMAGE .

echo "=== Pushing to ghcr.io ==="
docker push $IMAGE

echo "=== Done ==="
echo "Image: $IMAGE"
echo "Now update Render service to pull from: $IMAGE"
