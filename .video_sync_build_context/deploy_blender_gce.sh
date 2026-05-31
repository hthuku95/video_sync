#!/usr/bin/env bash
set -euo pipefail

# Deploy BlenderMCPServer to a small Compute Engine VM.
#
# Required:
#   GCP_PROJECT_ID
#
# Optional:
#   GCP_REGION                  default: us-central1
#   GCP_ZONE                    default: us-central1-a
#   BLENDER_GCE_INSTANCE        default: blender-mcp-vm
#   BLENDER_GCE_MACHINE_TYPE    default: e2-medium
#   BLENDER_GCE_DISK_SIZE       default: 80GB
#   BLENDER_GCE_ENV_FILE        default: ./blender_mcp.gce.env
#   BLENDER_GCE_STATIC_IP_NAME  default: blender-mcp-ip
#   BLENDER_GCE_FIREWALL_RULE   default: allow-blender-mcp-8000
#   BLENDER_GCE_TAG             default: blender-mcp-server

PROJECT_ID="${GCP_PROJECT_ID:?GCP_PROJECT_ID is required}"
REGION="${GCP_REGION:-us-central1}"
ZONE="${GCP_ZONE:-us-central1-a}"
INSTANCE_NAME="${BLENDER_GCE_INSTANCE:-blender-mcp-vm}"
MACHINE_TYPE="${BLENDER_GCE_MACHINE_TYPE:-e2-medium}"
DISK_SIZE="${BLENDER_GCE_DISK_SIZE:-80GB}"
ENV_FILE="${BLENDER_GCE_ENV_FILE:-./blender_mcp.gce.env}"
STATIC_IP_NAME="${BLENDER_GCE_STATIC_IP_NAME:-blender-mcp-ip}"
FIREWALL_RULE="${BLENDER_GCE_FIREWALL_RULE:-allow-blender-mcp-8000}"
NETWORK_TAG="${BLENDER_GCE_TAG:-blender-mcp-server}"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SERVER_DIR="${ROOT_DIR}/VideoSyncIntegrations/BlenderMCPServer"
TMP_DIR="${ROOT_DIR}/.blender_gce_tmp"
ARCHIVE_PATH="${TMP_DIR}/blender-mcp-server.tgz"
REMOTE_ARCHIVE="/tmp/blender-mcp-server.tgz"
REMOTE_ENV="/tmp/blender_mcp.gce.env"
REMOTE_SETUP="/tmp/setup_blender_mcp.sh"

if [[ ! -f "${ENV_FILE}" ]]; then
  echo "Missing env file: ${ENV_FILE}" >&2
  exit 1
fi

rm -rf "${TMP_DIR}"
mkdir -p "${TMP_DIR}"

tar \
  --exclude='.venv' \
  --exclude='__pycache__' \
  --exclude='.pytest_cache' \
  --exclude='*.pyc' \
  -czf "${ARCHIVE_PATH}" \
  -C "${SERVER_DIR}" .

cat > "${TMP_DIR}/setup_blender_mcp.sh" <<'EOF'
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
EOF

chmod +x "${TMP_DIR}/setup_blender_mcp.sh"

gcloud services enable \
  --project="${PROJECT_ID}" \
  compute.googleapis.com

if ! gcloud compute addresses describe "${STATIC_IP_NAME}" \
  --project="${PROJECT_ID}" \
  --region="${REGION}" >/dev/null 2>&1; then
  gcloud compute addresses create "${STATIC_IP_NAME}" \
    --project="${PROJECT_ID}" \
    --region="${REGION}"
fi

STATIC_IP="$(gcloud compute addresses describe "${STATIC_IP_NAME}" \
  --project="${PROJECT_ID}" \
  --region="${REGION}" \
  --format='value(address)')"

if ! gcloud compute firewall-rules describe "${FIREWALL_RULE}" \
  --project="${PROJECT_ID}" >/dev/null 2>&1; then
  gcloud compute firewall-rules create "${FIREWALL_RULE}" \
    --project="${PROJECT_ID}" \
    --allow=tcp:8000 \
    --target-tags="${NETWORK_TAG}" \
    --direction=INGRESS \
    --source-ranges=0.0.0.0/0
fi

if ! gcloud compute instances describe "${INSTANCE_NAME}" \
  --project="${PROJECT_ID}" \
  --zone="${ZONE}" >/dev/null 2>&1; then
  gcloud compute instances create "${INSTANCE_NAME}" \
    --project="${PROJECT_ID}" \
    --zone="${ZONE}" \
    --machine-type="${MACHINE_TYPE}" \
    --image-family=debian-12 \
    --image-project=debian-cloud \
    --boot-disk-size="${DISK_SIZE}" \
    --tags="${NETWORK_TAG}" \
    --address="${STATIC_IP}" \
    --metadata=enable-oslogin=FALSE
fi

gcloud compute scp \
  --project="${PROJECT_ID}" \
  --zone="${ZONE}" \
  "${ARCHIVE_PATH}" "${ENV_FILE}" "${TMP_DIR}/setup_blender_mcp.sh" \
  "${INSTANCE_NAME}:/tmp/"

gcloud compute ssh "${INSTANCE_NAME}" \
  --project="${PROJECT_ID}" \
  --zone="${ZONE}" \
  --command="sudo bash ${REMOTE_SETUP}"

echo "BlenderMCP instance: ${INSTANCE_NAME}"
echo "Static IP: ${STATIC_IP}"
echo "Health URL: http://${STATIC_IP}:8000/health"
