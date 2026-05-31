#!/usr/bin/env bash
set -euo pipefail

# Build and deploy the YTDLPAPI service to Cloud Run.
#
# Required environment variables:
#   GCP_PROJECT_ID
#   YTDLP_ENV_FILE             path to a YAML or .env-style file for Cloud Run env vars
#
# Optional environment variables:
#   GCP_REGION                 default: us-central1
#   YTDLP_SERVICE_NAME         default: ytdlp-service
#   YTDLP_ARTIFACT_REPO        default: videosync
#   YTDLP_IMAGE_TAG            default: latest
#   YTDLP_MEMORY               default: 2Gi
#   YTDLP_CPU                  default: 2
#   YTDLP_TIMEOUT              default: 3600
#   YTDLP_MAX_INSTANCES        default: 3
#   YTDLP_MIN_INSTANCES        default: 0
#   YTDLP_CONCURRENCY          default: 1

PROJECT_ID="${GCP_PROJECT_ID:?GCP_PROJECT_ID is required}"
ENV_FILE="${YTDLP_ENV_FILE:?YTDLP_ENV_FILE is required}"
REGION="${GCP_REGION:-us-central1}"
SERVICE_NAME="${YTDLP_SERVICE_NAME:-ytdlp-service}"
REPO_NAME="${YTDLP_ARTIFACT_REPO:-videosync}"
IMAGE_TAG="${YTDLP_IMAGE_TAG:-latest}"
MEMORY="${YTDLP_MEMORY:-2Gi}"
CPU="${YTDLP_CPU:-2}"
TIMEOUT="${YTDLP_TIMEOUT:-3600}"
MAX_INSTANCES="${YTDLP_MAX_INSTANCES:-3}"
MIN_INSTANCES="${YTDLP_MIN_INSTANCES:-0}"
CONCURRENCY="${YTDLP_CONCURRENCY:-1}"

if [[ ! -f "${ENV_FILE}" ]]; then
  echo "Missing env file: ${ENV_FILE}" >&2
  exit 1
fi

IMAGE_URI="${REGION}-docker.pkg.dev/${PROJECT_ID}/${REPO_NAME}/${SERVICE_NAME}:${IMAGE_TAG}"

echo "Using project: ${PROJECT_ID}"
echo "Using region:  ${REGION}"
echo "Using image:   ${IMAGE_URI}"
echo "Using envs:    ${ENV_FILE}"

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

gcloud builds submit VideoSyncIntegrations/YTDLPAPI \
  --project="${PROJECT_ID}" \
  --region="${REGION}" \
  --tag="${IMAGE_URI}"

gcloud run deploy "${SERVICE_NAME}" \
  --project="${PROJECT_ID}" \
  --region="${REGION}" \
  --image="${IMAGE_URI}" \
  --port=8000 \
  --memory="${MEMORY}" \
  --cpu="${CPU}" \
  --timeout="${TIMEOUT}" \
  --concurrency="${CONCURRENCY}" \
  --max-instances="${MAX_INSTANCES}" \
  --min-instances="${MIN_INSTANCES}" \
  --env-vars-file="${ENV_FILE}" \
  --allow-unauthenticated

gcloud run services describe "${SERVICE_NAME}" \
  --project="${PROJECT_ID}" \
  --region="${REGION}" \
  --format='value(status.url)'
