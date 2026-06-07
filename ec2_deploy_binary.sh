#!/bin/bash
# Deploy the freshly-built binary and restart the service.
set -e

EC2_IP="${EC2_IP:-3.224.144.201}"
KEY="/home/harry/projects/DevThukuDotIO/Rust/video_editor/.ssh-key-v6.pem"
REMOTE_DIR="/home/ubuntu/video_editor"

echo "=== Restarting service ==="
ssh -i "$KEY" -o StrictHostKeyChecking=no "ubuntu@$EC2_IP" \
  "sudo systemctl restart video-editor && echo 'Service restarted'"

echo "=== Checking service ==="
sleep 2
ssh -i "$KEY" -o StrictHostKeyChecking=no "ubuntu@$EC2_IP" \
  "sudo journalctl -u video-editor -n 10 --no-pager"

echo ""
echo "=== Done ==="