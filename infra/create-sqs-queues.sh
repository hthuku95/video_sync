#!/usr/bin/env bash
set -euo pipefail

# ─── Create SQS queues for the video-editor Batch worker pipeline ─────────────
# Requires: AWS CLI v2, credentials for us-east-1
# Usage: ./create-sqs-queues.sh
#
# This creates two queues per job type:
#   1. Main queue — Batch workers pull messages from this
#   2. Dead-letter queue — failed messages land here after 3 retries

REGION="us-east-1"
ACCOUNT_ID=$(aws sts get-caller-identity --query Account --output text)

# Clipping jobs queue
DLQ_NAME="video-editor-clipping-dlq"
QUEUE_NAME="video-editor-clipping"

echo "=== Creating SQS queues in ${REGION} ==="

# Create DLQ first
DLQ_URL=$(aws sqs create-queue \
    --region "${REGION}" \
    --queue-name "${DLQ_NAME}" \
    --attributes '{
        "MessageRetentionPeriod": "1209600"
    }' \
    --query 'QueueUrl' --output text)

DLQ_ARN=$(aws sqs get-queue-attributes \
    --region "${REGION}" \
    --queue-url "${DLQ_URL}" \
    --attribute-names QueueArn \
    --query 'Attributes.QueueArn' --output text)

echo "✅ DLQ created: ${DLQ_NAME} (ARN: ${DLQ_ARN})"

# Create main queue with DLQ redrive policy
MAIN_QUEUE_URL=$(aws sqs create-queue \
    --region "${REGION}" \
    --queue-name "${QUEUE_NAME}" \
    --attributes '{
        "VisibilityTimeout": "3600",
        "MessageRetentionPeriod": "345600",
        "ReceiveMessageWaitTimeSeconds": "20",
        "RedrivePolicy": "{\"deadLetterTargetArn\":\"'"${DLQ_ARN}"'\",\"maxReceiveCount\":\"3\"}"
    }' \
    --query 'QueueUrl' --output text)

MAIN_QUEUE_ARN=$(aws sqs get-queue-attributes \
    --region "${REGION}" \
    --queue-url "${MAIN_QUEUE_URL}" \
    --attribute-names QueueArn \
    --query 'Attributes.QueueArn' --output text)

echo "✅ Main queue created: ${QUEUE_NAME} (ARN: ${MAIN_QUEUE_ARN})"
echo ""
echo "=== Queue URLs ==="
echo "Main: ${MAIN_QUEUE_URL}"
echo "DLQ:  ${DLQ_URL}"
echo ""
echo "Set these env vars on the Rust API server:"
echo "  CLIPPING_SQS_QUEUE_URL=${MAIN_QUEUE_URL}"
