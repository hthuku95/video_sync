#!/bin/bash
set -e

if [ -n "$S3_ENV_FILE" ]; then
    echo "entrypoint: downloading env file from $S3_ENV_FILE"
    AWS_PAGER="" aws s3 cp "$S3_ENV_FILE" /app/.env --sse AES256 2>&1
    RC=$?
    if [ $RC -eq 0 ]; then
        echo "entrypoint: env file downloaded successfully"
    else
        echo "entrypoint: FAILED to download env file (exit code $RC)"
        echo "entrypoint: checking AWS credentials..."
        AWS_PAGER="" aws sts get-caller-identity 2>&1 || echo "entrypoint: no valid credentials"
    fi
fi

if [ -f /app/.env ]; then
    echo "entrypoint: loading /app/.env"
    set -a
    . /app/.env
    set +a
    echo "entrypoint: DATABASE_URL=${DATABASE_URL:+SET (${DATABASE_URL:0:30}...)}${DATABASE_URL:-NOT SET}"
    echo "entrypoint: GEMINI_API_KEY=${GEMINI_API_KEY:+SET (${GEMINI_API_KEY:0:15}...)}${GEMINI_API_KEY:-NOT SET}"
    echo "entrypoint: R2_BUCKET=${R2_BUCKET:-NOT SET}"
else
    echo "entrypoint: /app/.env NOT FOUND"
fi

if [ "$BATCH_MODE" = "true" ]; then
    echo "entrypoint: BATCH_MODE=true - processing one SQS message"
fi

exec "$@"
