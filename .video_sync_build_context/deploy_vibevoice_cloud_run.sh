#!/usr/bin/env bash
set -euo pipefail

# Deploy the VideoSync VibeVoice service to Cloud Run using the
# VideoSyncIntegrations Docker context.
#
# Required environment variables:
#   GCP_PROJECT_ID
#
# Optional environment variables:
#   GCP_REGION                 default: us-central1
#   VIBEVOICE_SERVICE_NAME     default: videosync-vibevoice
#   VIBEVOICE_ARTIFACT_REPO    default: videosync
#   VIBEVOICE_IMAGE_TAG        default: latest
#   VIBEVOICE_MEMORY           default: 8Gi
#   VIBEVOICE_CPU              default: 4
#   VIBEVOICE_TIMEOUT          default: 3600
#   VIBEVOICE_MAX_INSTANCES    default: 2
#   VIBEVOICE_MIN_INSTANCES    default: 0
#   VIBEVOICE_CONCURRENCY      default: 1
#   VIBEVOICE_TTS_MODEL        default: microsoft/VibeVoice-Realtime-0.5B
#   VIBEVOICE_ASR_MODEL        default: microsoft/VibeVoice-ASR-7B
#   VIBEVOICE_TTS_CPU_DTYPE    default: bfloat16

PROJECT_ID="${GCP_PROJECT_ID:?GCP_PROJECT_ID is required}"
REGION="${GCP_REGION:-us-central1}"
SERVICE_NAME="${VIBEVOICE_SERVICE_NAME:-videosync-vibevoice}"
REPO_NAME="${VIBEVOICE_ARTIFACT_REPO:-videosync}"
IMAGE_TAG="${VIBEVOICE_IMAGE_TAG:-latest}"
MEMORY="${VIBEVOICE_MEMORY:-8Gi}"
CPU="${VIBEVOICE_CPU:-4}"
TIMEOUT="${VIBEVOICE_TIMEOUT:-3600}"
MAX_INSTANCES="${VIBEVOICE_MAX_INSTANCES:-2}"
MIN_INSTANCES="${VIBEVOICE_MIN_INSTANCES:-0}"
CONCURRENCY="${VIBEVOICE_CONCURRENCY:-1}"
TTS_MODEL="${VIBEVOICE_TTS_MODEL:-microsoft/VibeVoice-Realtime-0.5B}"
ASR_MODEL="${VIBEVOICE_ASR_MODEL:-microsoft/VibeVoice-ASR-7B}"
TTS_CPU_DTYPE="${VIBEVOICE_TTS_CPU_DTYPE:-bfloat16}"

IMAGE_URI="${REGION}-docker.pkg.dev/${PROJECT_ID}/${REPO_NAME}/${SERVICE_NAME}:${IMAGE_TAG}"

echo "Using project: ${PROJECT_ID}"
echo "Using region:  ${REGION}"
echo "Using image:   ${IMAGE_URI}"

gcloud services enable \
  --project="${PROJECT_ID}" \
  run.googleapis.com \
  cloudbuild.googleapis.com \
  artifactregistry.googleapis.com

if ! gcloud artifacts repositories describe "${REPO_NAME}" \
  --location="${REGION}" \
  --project="${PROJECT_ID}" >/dev/null 2>&1; then
  gcloud artifacts repositories create "${REPO_NAME}" \
    --location="${REGION}" \
    --project="${PROJECT_ID}" \
    --repository-format=docker
fi

gcloud builds submit VideoSyncIntegrations \
  --project="${PROJECT_ID}" \
  --region="${REGION}" \
  --config=cloudbuild.vibevoice.yaml

gcloud run deploy "${SERVICE_NAME}" \
  --project="${PROJECT_ID}" \
  --region="${REGION}" \
  --image="${IMAGE_URI}" \
  --port=8015 \
  --memory="${MEMORY}" \
  --cpu="${CPU}" \
  --timeout="${TIMEOUT}" \
  --concurrency="${CONCURRENCY}" \
  --max-instances="${MAX_INSTANCES}" \
  --min-instances="${MIN_INSTANCES}" \
  --set-env-vars="VIBEVOICE_OUTPUT_DIR=/tmp/videosync-vibevoice,VIBEVOICE_TTS_MODEL=${TTS_MODEL},VIBEVOICE_ASR_MODEL=${ASR_MODEL},VIBEVOICE_TTS_CPU_DTYPE=${TTS_CPU_DTYPE}" \
  --allow-unauthenticated

gcloud run services describe "${SERVICE_NAME}" \
  --project="${PROJECT_ID}" \
  --region="${REGION}" \
  --format='value(status.url)'
