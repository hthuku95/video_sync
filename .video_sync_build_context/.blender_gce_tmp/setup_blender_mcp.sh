#!/usr/bin/env bash
set -euo pipefail

export DEBIAN_FRONTEND=noninteractive

apt-get update
apt-get install -y --no-install-recommends \
  build-essential \
  python3 \
  python3-dev \
  python3-venv \
  python3-pip \
  meson \
  ninja-build \
  blender \
  libgl1 \
  libglib2.0-0 \
  libgomp1 \
  libegl1 \
  xvfb \
  xauth \
  texlive-latex-base \
  texlive-fonts-recommended \
  texlive-latex-extra \
  texlive-science \
  texlive-fonts-extra \
  dvisvgm \
  dvipng \
  libcairo2 \
  libpango-1.0-0 \
  libpangocairo-1.0-0 \
  libcairo2-dev \
  libpango1.0-dev \
  pkg-config \
  ffmpeg \
  curl

mkdir -p /opt/blender-mcp-server
tar -xzf /tmp/blender-mcp-server.tgz -C /opt/blender-mcp-server

python3 -m venv /opt/blender-mcp-server/.venv
/opt/blender-mcp-server/.venv/bin/pip install --upgrade pip
/opt/blender-mcp-server/.venv/bin/pip install --no-cache-dir -r /opt/blender-mcp-server/requirements.txt

install -m 600 /tmp/blender_mcp.gce.env /etc/blender-mcp.env

cat > /etc/systemd/system/blender-mcp.service <<'SERVICE'
[Unit]
Description=Blender MCP Server
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=root
WorkingDirectory=/opt/blender-mcp-server
EnvironmentFile=/etc/blender-mcp.env
ExecStart=/opt/blender-mcp-server/.venv/bin/python /opt/blender-mcp-server/server.py
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
SERVICE

systemctl daemon-reload
systemctl enable blender-mcp.service
systemctl restart blender-mcp.service
systemctl --no-pager --full status blender-mcp.service || true
