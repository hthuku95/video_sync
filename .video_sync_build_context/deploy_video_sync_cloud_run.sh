#!/usr/bin/env bash
set -euo pipefail

# Build and deploy the Rust `video_sync` service from this repository to Cloud Run.
#
# Required environment variables:
#   GCP_PROJECT_ID
#   VIDEO_SYNC_ENV_FILE        path to a YAML or .env-style file for Cloud Run env vars
#
# Optional environment variables:
#   GCP_REGION                 default: us-central1
#   VIDEO_SYNC_SERVICE_NAME    default: video-sync
#   VIDEO_SYNC_ARTIFACT_REPO   default: videosync
#   VIDEO_SYNC_IMAGE_TAG       default: latest
#   VIDEO_SYNC_MEMORY          default: 4Gi
#   VIDEO_SYNC_CPU             default: 2
#   VIDEO_SYNC_TIMEOUT         default: 3600
#   VIDEO_SYNC_MAX_INSTANCES   default: 3
#   VIDEO_SYNC_MIN_INSTANCES   default: 0
#   VIDEO_SYNC_CONCURRENCY     default: 4
#   VIDEO_SYNC_CPU_THROTTLING  default: false (keeps background workers/workflows running after HTTP responses)
#   VIDEO_SYNC_CPU_BOOST       default: true
#   VIDEO_SYNC_BUILD_MACHINE_TYPE optional Cloud Build machine type override
#   VIDEO_SYNC_BUILD_TIMEOUT   default: 7200s
#   VIDEO_SYNC_BUILD_CONTEXT   default: .video_sync_build_context

PROJECT_ID="${GCP_PROJECT_ID:?GCP_PROJECT_ID is required}"
ENV_FILE="${VIDEO_SYNC_ENV_FILE:?VIDEO_SYNC_ENV_FILE is required}"
REGION="${GCP_REGION:-us-central1}"
SERVICE_NAME="${VIDEO_SYNC_SERVICE_NAME:-video-sync}"
REPO_NAME="${VIDEO_SYNC_ARTIFACT_REPO:-videosync}"
IMAGE_TAG="${VIDEO_SYNC_IMAGE_TAG:-latest}"
MEMORY="${VIDEO_SYNC_MEMORY:-4Gi}"
CPU="${VIDEO_SYNC_CPU:-2}"
TIMEOUT="${VIDEO_SYNC_TIMEOUT:-3600}"
MAX_INSTANCES="${VIDEO_SYNC_MAX_INSTANCES:-3}"
MIN_INSTANCES="${VIDEO_SYNC_MIN_INSTANCES:-0}"
CONCURRENCY="${VIDEO_SYNC_CONCURRENCY:-4}"
CPU_THROTTLING="${VIDEO_SYNC_CPU_THROTTLING:-false}"
CPU_BOOST="${VIDEO_SYNC_CPU_BOOST:-true}"
BUILD_MACHINE_TYPE="${VIDEO_SYNC_BUILD_MACHINE_TYPE:-}"
BUILD_TIMEOUT="${VIDEO_SYNC_BUILD_TIMEOUT:-7200s}"
BUILD_CONTEXT="${VIDEO_SYNC_BUILD_CONTEXT:-.video_sync_build_context}"

if [[ ! -f "${ENV_FILE}" ]]; then
  echo "Missing env file: ${ENV_FILE}" >&2
  exit 1
fi

IMAGE_URI="${REGION}-docker.pkg.dev/${PROJECT_ID}/${REPO_NAME}/${SERVICE_NAME}:${IMAGE_TAG}"

echo "Using project: ${PROJECT_ID}"
echo "Using region:  ${REGION}"
echo "Using image:   ${IMAGE_URI}"
echo "Using envs:    ${ENV_FILE}"

echo "Preparing minimal Cloud Build context: ${BUILD_CONTEXT}"
rm -rf "${BUILD_CONTEXT}"
mkdir -p "${BUILD_CONTEXT}"
cp Cargo.toml Cargo.lock Dockerfile .dockerignore .gcloudignore "${BUILD_CONTEXT}/"
cp -R src migrations "${BUILD_CONTEXT}/"
du -sh "${BUILD_CONTEXT}" || true

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

BUILD_SUBMIT_ARGS=(
  "${BUILD_CONTEXT}"
  "--project=${PROJECT_ID}"
  "--region=${REGION}"
  "--tag=${IMAGE_URI}"
  "--timeout=${BUILD_TIMEOUT}"
)

if [[ -n "${BUILD_MACHINE_TYPE}" ]]; then
  BUILD_SUBMIT_ARGS+=("--machine-type=${BUILD_MACHINE_TYPE}")
fi

gcloud builds submit "${BUILD_SUBMIT_ARGS[@]}"

gcloud run deploy "${SERVICE_NAME}" \
  --project="${PROJECT_ID}" \
  --region="${REGION}" \
  --image="${IMAGE_URI}" \
  --port=3000 \
  --memory="${MEMORY}" \
  --cpu="${CPU}" \
  --timeout="${TIMEOUT}" \
  --concurrency="${CONCURRENCY}" \
  --max-instances="${MAX_INSTANCES}" \
  --min-instances="${MIN_INSTANCES}" \
  --env-vars-file="${ENV_FILE}" \
  --allow-unauthenticated \
  "$([[ "${CPU_THROTTLING}" == "true" ]] && echo "--cpu-throttling" || echo "--no-cpu-throttling")" \
  "$([[ "${CPU_BOOST}" == "false" ]] && echo "--no-cpu-boost" || echo "--cpu-boost")"

gcloud run services describe "${SERVICE_NAME}" \
  --project="${PROJECT_ID}" \
  --region="${REGION}" \
  --format='value(status.url)'
