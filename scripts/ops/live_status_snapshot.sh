#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

DATABASE_URL="$(grep -m1 '^DATABASE_URL=' .env | cut -d= -f2-)"

if [[ -z "${DATABASE_URL:-}" ]]; then
  echo "DATABASE_URL missing from .env" >&2
  exit 1
fi

echo "=== clipping_jobs all ==="
psql "$DATABASE_URL" -Atc "SELECT status, COUNT(*) FROM clipping_jobs GROUP BY status ORDER BY status;"

echo
echo "=== clipping_jobs last_24h ==="
psql "$DATABASE_URL" -Atc "SELECT status, COUNT(*) FROM clipping_jobs WHERE created_at >= NOW() - INTERVAL '24 hours' GROUP BY status ORDER BY status;"

echo
echo "=== clipping pending older_than_15m ==="
psql "$DATABASE_URL" -Atc "SELECT COUNT(*) FROM clipping_jobs WHERE status = 'pending' AND created_at < NOW() - INTERVAL '15 minutes';"

echo
echo "=== clipping recent failures ==="
psql "$DATABASE_URL" -Atc "SELECT id, status, COALESCE(left(error_message, 220), '') FROM clipping_jobs WHERE status IN ('failed','cancelled') ORDER BY updated_at DESC NULLS LAST, created_at DESC LIMIT 12;"

echo
echo "=== clipping latest completed ==="
psql "$DATABASE_URL" -Atc "SELECT id, status, created_at, updated_at FROM clipping_jobs WHERE status = 'completed' ORDER BY COALESCE(updated_at, created_at) DESC LIMIT 10;"

echo
echo "=== clipping fallback deliveries ==="
psql "$DATABASE_URL" -Atc "SELECT id, fallback_delivery_id, status, created_at, updated_at, COALESCE(left(error_message, 220), '') FROM clipping_jobs WHERE fallback_delivery_id IS NOT NULL ORDER BY COALESCE(updated_at, created_at) DESC LIMIT 10;"

echo
echo "=== phantombuster recent jobs ==="
psql "$DATABASE_URL" -Atc "SELECT id, COALESCE(agent_name,''), status, COALESCE(left(error, 220), ''), launched_at, completed_at FROM phantombuster_jobs ORDER BY created_at DESC LIMIT 12;"

echo
echo "=== phantombuster failed_or_pending ==="
psql "$DATABASE_URL" -Atc "SELECT id, COALESCE(agent_name,''), status, COALESCE(left(error, 220), ''), launched_at, completed_at FROM phantombuster_jobs WHERE status IN ('failed','pending','running') ORDER BY created_at DESC LIMIT 20;"

echo
echo "=== phantombuster linkedin_recent ==="
psql "$DATABASE_URL" -Atc "SELECT id, COALESCE(agent_name,''), status, COALESCE(left(search_url, 220), ''), COALESCE(left(error, 220), ''), launched_at, completed_at FROM phantombuster_jobs WHERE agent_name ILIKE '%LinkedIn%' OR agent_name ILIKE '%Sales Navigator%' ORDER BY created_at DESC LIMIT 12;"
