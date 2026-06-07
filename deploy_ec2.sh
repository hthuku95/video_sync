#!/bin/bash
# Deploy to EC2: syncs sources, starts build in persistent tmux session.
set -e

EC2_IP="${EC2_IP:-3.224.144.201}"
KEY="/home/harry/projects/DevThukuDotIO/Rust/video_editor/.ssh-key-v6.pem"
REMOTE_DIR="/home/ubuntu/video_editor"
LOCAL_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "=== 1/4: Syncing source files ==="
rsync -az --delete \
  --exclude 'target/' --exclude '.git/' \
  --exclude 'VideoSyncIntegrations/' --exclude 'env/' \
  --exclude '*.mp4' --exclude '*.jpg' --exclude '*.png' \
  --exclude '__pycache__/' --exclude '*.pyc' \
  -e "ssh -i $KEY -o StrictHostKeyChecking=no" \
  "$LOCAL_DIR/" "ubuntu@$EC2_IP:$REMOTE_DIR/"

echo "=== 2/4: Cleaning stale build processes ==="
ssh -i "$KEY" -o StrictHostKeyChecking=no "ubuntu@$EC2_IP" \
  "pkill -9 -f 'cargo\|rustc' 2>/dev/null; rm -f $REMOTE_DIR/target/.cargo-lock; echo done"

echo "=== 3/4: Starting build in tmux session 'build' ==="
ssh -i "$KEY" -o StrictHostKeyChecking=no "ubuntu@$EC2_IP" \
  "tmux new-session -d -s build 2>/dev/null; \
   tmux send-keys -t build \"export PATH=\\\$HOME/.cargo/bin:\\\$PATH && cd $REMOTE_DIR && cargo build --release 2>&1 | tee $REMOTE_DIR/build.log\" Enter"

echo ""
echo "=== 4/4: Build is running in background ==="
echo ""
echo "  Check progress:  ./ec2_status.sh"
echo "  Live tail:       ./ec2_tail.sh"
echo "  Attach to tmux:  ssh -t -i $KEY ubuntu@$EC2_IP \"tmux attach -t build\""
echo "  Detach from tmux: Ctrl+B, D"
echo "  Deploy after build: ./ec2_deploy_binary.sh"
