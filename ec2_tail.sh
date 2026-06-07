#!/bin/bash
# Live-tail build progress on EC2
EC2_IP="${EC2_IP:-3.224.144.201}"
KEY="/home/harry/projects/DevThukuDotIO/Rust/video_editor/.ssh-key-v6.pem"
ssh -i "$KEY" -o StrictHostKeyChecking=no "ubuntu@$EC2_IP" "tail -f /home/ubuntu/video_editor/build.log"