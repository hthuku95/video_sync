#!/usr/bin/env bash
set -euo pipefail

# ─── Create AWS Batch compute environment + job definition ────────────────────
# Requires: AWS CLI v2, Docker image pushed to ECR
# Usage: ./create-batch.sh <ecr-repo-uri> [image-tag]
#
# Prerequisites:
#   1. Build & push Docker image to ECR:
#      aws ecr create-repository --repository-name video-editor
#      docker build -t video-editor .
#      docker tag video-editor:latest <account>.dkr.ecr.us-east-1.amazonaws.com/video-editor:latest
#      docker push <account>.dkr.ecr.us-east-1.amazonaws.com/video-editor:latest
#   2. Create SQS queues first (run create-sqs-queues.sh)

REGION="us-east-1"
ECR_URI="${1:?Usage: $0 <ecr-repo-uri> [image-tag]}"
IMAGE_TAG="${2:-latest}"
IMAGE="${ECR_URI}:${IMAGE_TAG}"

# IAM role for Batch jobs (must exist)
BATCH_ROLE_ARN="arn:aws:iam::$(aws sts get-caller-identity --query Account --output text):role/aws-batch-service-role"
JOB_ROLE_ARN="arn:aws:iam::$(aws sts get-caller-identity --query Account --output text):role/ecs-task-execution-role"

echo "=== Creating AWS Batch resources in ${REGION} ==="

# ── Compute Environment (Graviton Spot) ──────────────────────────────────
CE_NAME="video-editor-clipping-spot"

# Check if compute environment already exists
EXISTING_CE=$(aws batch describe-compute-environments \
    --compute-environments "${CE_NAME}" \
    --query 'computeEnvironments[0].computeEnvironmentArn' --output text 2>/dev/null || true)

if [[ "${EXISTING_CE}" == "None" || -z "${EXISTING_CE}" ]]; then
    echo "Creating compute environment: ${CE_NAME}"

    aws batch create-compute-environment \
        --compute-environment-name "${CE_NAME}" \
        --type MANAGED \
        --state ENABLED \
        --service-role "${BATCH_ROLE_ARN}" \
        --compute-resources '{
            "type": "SPOT",
            "allocationStrategy": "SPOT_CAPACITY_OPTIMIZED",
            "minvCpus": 0,
            "desiredvCpus": 0,
            "maxvCpus": 256,
            "instanceTypes": ["c7g.medium","c7g.large","c7g.xlarge","c7g.2xlarge"],
            "subnets": ["subnet-07c812363b1d962d0"],
            "securityGroupIds": ["sg-0437b87ed5e793766"],
            "instanceRole": "ecs-svc-role-aws-batch",
            "tags": {
                "Project": "video-editor",
                "Environment": "production"
            }
        }'

    echo "⏳ Waiting for compute environment to become valid..."
    aws batch wait compute-environment-valid --compute-environments "${CE_NAME}"
    echo "✅ Compute environment created: ${CE_NAME}"
else
    echo "✅ Compute environment already exists: ${CE_NAME}"
fi

# ── Job Queue ─────────────────────────────────────────────────────────────
JOB_QUEUE_NAME="video-editor-clipping-queue"

EXISTING_JQ=$(aws batch describe-job-queues \
    --job-queues "${JOB_QUEUE_NAME}" \
    --query 'jobQueues[0].jobQueueArn' --output text 2>/dev/null || true)

if [[ "${EXISTING_JQ}" == "None" || -z "${EXISTING_JQ}" ]]; then
    CE_ARN=$(aws batch describe-compute-environments \
        --compute-environments "${CE_NAME}" \
        --query 'computeEnvironments[0].computeEnvironmentArn' --output text)

    aws batch create-job-queue \
        --job-queue-name "${JOB_QUEUE_NAME}" \
        --state ENABLED \
        --priority 1 \
        --compute-environment-order "order=1,computeEnvironment=${CE_ARN}"

    echo "✅ Job queue created: ${JOB_QUEUE_NAME}"
else
    echo "✅ Job queue already exists: ${JOB_QUEUE_NAME}"
fi

# ── Job Definition ────────────────────────────────────────────────────────
JOB_DEF_NAME="video-editor-clipping"

EXISTING_JD=$(aws batch describe-job-definitions \
    --job-definition-name "${JOB_DEF_NAME}" \
    --status ACTIVE \
    --query 'jobDefinitions[0].jobDefinitionArn' --output text 2>/dev/null || true)

if [[ "${EXISTING_JD}" == "None" || -z "${EXISTING_JD}" ]]; then
    echo "Creating job definition: ${JOB_DEF_NAME}"

    aws batch register-job-definition \
        --job-definition-name "${JOB_DEF_NAME}" \
        --type container \
        --container-properties '{
            "image": "'"${IMAGE}"'",
            "command": ["video_editor"],
            "jobRoleArn": "'"${JOB_ROLE_ARN}"'",
            "environment": [
                {"name": "WORKER_MODE", "value": "true"}
            ],
            "resourceRequirements": [
                {"type": "VCPU", "value": "2"},
                {"type": "MEMORY", "value": "4096"}
            ],
            "executionRoleArn": "'"${JOB_ROLE_ARN}"'",
            "logConfiguration": {
                "logDriver": "awslogs",
                "options": {
                    "awslogs-group": "/aws/batch/video-editor",
                    "awslogs-region": "'"${REGION}"'",
                    "awslogs-stream-prefix": "clipping"
                }
            }
        }' \
        --retry-strategy '{
            "attempts": 3,
            "evaluateOnExit": [
                {"onStatusReason": ".*", "action": "RETRY"}
            ]
        }'

    echo "✅ Job definition created: ${JOB_DEF_NAME}"
else
    echo "✅ Job definition already exists: ${JOB_DEF_NAME}"
fi

echo ""
echo "=== Summary ==="
echo "Compute Environment: ${CE_NAME}"
echo "Job Queue:          ${JOB_QUEUE_NAME}"
echo "Job Definition:     ${JOB_DEF_NAME}"
echo "Image:              ${IMAGE}"
echo ""
echo "To submit a test job:"
echo "  aws batch submit-job \\"
echo "    --job-name test-clipping-1 \\"
echo "    --job-queue ${JOB_QUEUE_NAME} \\"
echo "    --job-definition ${JOB_DEF_NAME}"
echo ""
echo "To submit via SQS → Batch (EventBridge Pipes):"
echo "  1. Create EventBridge Pipe from SQS queue → Batch job queue"
echo "  2. Pipe extracts job_id from SQS message, passes as env var"
