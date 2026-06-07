#!/bin/bash
# Check EC2 build progress
EC2_IP="${EC2_IP:-3.224.144.201}"
KEY="/home/harry/projects/DevThukuDotIO/Rust/video_editor/.ssh-key-v6.pem"
ssh -i "$KEY" -o StrictHostKeyChecking=no "ubuntu@$EC2_IP" "tail -20 /home/ubuntu/video_editor/build.log 2>/dev/null || echo 'No build log found'"
echo "---"
ssh -i "$KEY" -o StrictHostKeyChecking=no "ubuntu@$EC2_IP" "tmux ls 2>/dev/null || echo 'No tmux sessions'"
echo "---"
ssh -i "$KEY" -o StrictHostKeyChecking=no "ubuntu@$EC2_IP" "ls -lh /home/ubuntu/video_editor/target/debug/video_editor 2>/dev/null || echo 'Binary not built yet'"