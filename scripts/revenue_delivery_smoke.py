#!/usr/bin/env python3
"""Create and inspect production revenue delivery smoke jobs.

This intentionally avoids printing secrets. It reads the Cloud Run env YAML,
creates a short-lived admin JWT, and calls the deployed admin deliveries API.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import hmac
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path


DEFAULT_BASE_URL = "https://video-sync-723463981172.us-central1.run.app"


def parse_env_yaml(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#") or ":" not in line:
            continue
        key, value = line.split(":", 1)
        value = value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in {"'", '"'}:
            value = value[1:-1]
        values[key.strip()] = value
    return values


def sql_literal(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def load_admin_user(config: dict[str, str]) -> dict[str, object]:
    database_url = config.get("DATABASE_URL")
    admin_email = config.get("TEST_ADMIN_EMAIL")
    if not database_url or not admin_email:
        raise RuntimeError("DATABASE_URL or TEST_ADMIN_EMAIL is missing from env YAML")

    env = os.environ.copy()
    env["DATABASE_URL"] = database_url
    sql = (
        "SELECT id, COALESCE(username, ''), email, is_superuser, is_staff, is_clipper "
        f"FROM users WHERE email = {sql_literal(admin_email)} LIMIT 1;"
    )
    result = subprocess.run(
        ["psql", database_url, "-At", "-F", "\t", "-c", sql],
        env=env,
        text=True,
        capture_output=True,
        timeout=30,
        check=False,
    )
    if result.returncode != 0:
        safe_stderr = result.stderr.replace(database_url, "[DATABASE_URL]")
        raise RuntimeError(f"Unable to query admin user with psql: {safe_stderr.splitlines()[-1] if safe_stderr else 'no stderr'}")
    line = result.stdout.strip()
    if not line:
        raise RuntimeError("Admin user was not found")
    user_id, username, email, is_superuser, is_staff, is_clipper = line.split("\t")
    return {
        "sub": user_id,
        "username": username or email.split("@", 1)[0],
        "email": email,
        "is_superuser": is_superuser == "t",
        "is_staff": is_staff == "t",
        "is_clipper": is_clipper == "t",
    }


def b64url(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).decode("ascii").rstrip("=")


def mint_jwt(config: dict[str, str], user: dict[str, object]) -> str:
    secret = config.get("JWT_SECRET")
    if not secret:
        raise RuntimeError("JWT_SECRET is missing from env YAML")
    now = int(time.time())
    claims = {
        **user,
        "iat": now,
        "exp": now + 60 * 60,
    }
    header = {"alg": "HS256", "typ": "JWT"}
    signing_input = ".".join(
        [
            b64url(json.dumps(header, separators=(",", ":")).encode("utf-8")),
            b64url(json.dumps(claims, separators=(",", ":")).encode("utf-8")),
        ]
    ).encode("ascii")
    signature = hmac.new(secret.encode("utf-8"), signing_input, hashlib.sha256).digest()
    return signing_input.decode("ascii") + "." + b64url(signature)


def api_request(base_url: str, token: str, method: str, path: str, body: object | None = None) -> object:
    data = None
    headers = {"Authorization": f"Bearer {token}"}
    if body is not None:
        data = json.dumps(body).encode("utf-8")
        headers["Content-Type"] = "application/json"
    request = urllib.request.Request(base_url.rstrip("/") + path, data=data, method=method, headers=headers)
    try:
        with urllib.request.urlopen(request, timeout=120) as response:
            return json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as exc:
        detail = exc.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"API returned HTTP {exc.code}: {detail[:500]}") from exc


def create_delivery(base_url: str, token: str, payload: dict[str, object]) -> None:
    result = api_request(base_url, token, "POST", "/api/admin/deliveries", payload)
    print(
        json.dumps(
            {
                "created": True,
                "delivery_id": result.get("delivery_id"),
                "workflow_id": result.get("workflow_id"),
                "title": payload["title"],
                "gig_type": payload["gig_type"],
            },
            indent=2,
        )
    )


def list_deliveries(base_url: str, token: str, limit: int) -> None:
    result = api_request(base_url, token, "GET", "/api/admin/deliveries")
    deliveries = result.get("deliveries", [])[:limit]
    safe_rows = [
        {
            "id": row.get("id"),
            "title": row.get("title"),
            "gig_type": row.get("gig_type"),
            "status": row.get("status"),
            "workflow_id": row.get("workflow_id"),
            "output": bool(row.get("output_r2_url")),
            "error": row.get("error_message"),
        }
        for row in deliveries
    ]
    print(json.dumps({"deliveries": safe_rows}, indent=2))


def query_workflows(config: dict[str, str], workflow_id: str | None, limit: int) -> None:
    database_url = config.get("DATABASE_URL")
    if not database_url:
        raise RuntimeError("DATABASE_URL is missing from env YAML")

    where = f"WHERE id = {sql_literal(workflow_id)}" if workflow_id else ""
    sql = (
        "SELECT json_build_object("
        "'id', id::text, "
        "'workflow_type', workflow_type, "
        "'status', status, "
        "'current_step', current_step, "
        "'summary', request_summary, "
        "'error', error_message, "
        "'last_heartbeat_at', last_heartbeat_at, "
        "'updated_at', updated_at"
        ")::text "
        f"FROM app_workflows {where} ORDER BY created_at DESC LIMIT {int(limit)};"
    )
    result = subprocess.run(
        ["psql", database_url, "-At", "-F", "\t", "-c", sql],
        text=True,
        capture_output=True,
        timeout=30,
        check=False,
    )
    if result.returncode != 0:
        safe_stderr = result.stderr.replace(database_url, "[DATABASE_URL]")
        raise RuntimeError(f"Unable to query workflows: {safe_stderr.splitlines()[-1] if safe_stderr else 'no stderr'}")

    rows = []
    for line in result.stdout.splitlines():
        rows.append(json.loads(line))
    print(json.dumps({"workflows": rows}, indent=2))


def query_workflow_events(config: dict[str, str], workflow_id: str, limit: int) -> None:
    database_url = config.get("DATABASE_URL")
    if not database_url:
        raise RuntimeError("DATABASE_URL is missing from env YAML")
    sql = (
        "SELECT event_type, COALESCE(node_name, ''), message, COALESCE(details::text, '{}'), created_at::text "
        "FROM app_workflow_events "
        f"WHERE workflow_id = {sql_literal(workflow_id)} "
        f"ORDER BY created_at DESC LIMIT {int(limit)};"
    )
    result = subprocess.run(
        ["psql", database_url, "-At", "-F", "\t", "-c", sql],
        text=True,
        capture_output=True,
        timeout=30,
        check=False,
    )
    if result.returncode != 0:
        safe_stderr = result.stderr.replace(database_url, "[DATABASE_URL]")
        raise RuntimeError(f"Unable to query workflow events: {safe_stderr.splitlines()[-1] if safe_stderr else 'no stderr'}")

    events = []
    for line in result.stdout.splitlines():
        parts = line.split("\t", 4)
        if len(parts) != 5:
            continue
        event_type, node_name, message, details, created_at = parts
        events.append(
            {
                "event_type": event_type,
                "node_name": node_name,
                "message": message,
                "details": details[:500],
                "created_at": created_at,
            }
        )
    print(json.dumps({"events": events}, indent=2))


def sample_payloads() -> list[dict[str, object]]:
    return [
        {
            "client_ref": "revenue-v1-smoke-saas-demo",
            "title": "Revenue V1 SaaS Demo Pack - InvoiceFlow AI",
            "gig_type": "long_form_video",
            "prompt": (
                "Create a paid-client style sample for a fictional B2B SaaS called InvoiceFlow AI. "
                "Show a CFO pain point, a clean app dashboard concept, automated invoice reconciliation, "
                "and a strong 24-hour promo-video offer. Include reusable outreach hooks and a polished CTA."
            ),
            "style": "premium SaaS launch video, clean UI mockups, kinetic captions, confident narration",
            "duration": 45,
            "extra": {
                "offer_type": "landing_page",
                "service_offer": "saas_demo_pack",
                "segment_duration_seconds": 15,
                "include_narration": True,
                "narration_speaker": "Emma",
                "reference_url": "https://videosync.video",
            },
        },
        {
            "client_ref": "revenue-v1-smoke-agency-bundle",
            "title": "Revenue V1 Mixed Agency Bundle - Creator Growth Studio",
            "gig_type": "long_form_video",
            "prompt": (
                "Create a short proof-of-capability sample for a creator-growth agency package. "
                "Combine clip-pack positioning, thumbnail/hero visual concept, voiceover, productized pricing, "
                "and a delivery-page CTA aimed at creators or indie founders who need content fast."
            ),
            "style": "bold agency reel, social proof energy, split-screen artifacts, upbeat narration",
            "duration": 45,
            "extra": {
                "offer_type": "agency_bundle",
                "service_offer": "agency_bundle_pack",
                "segment_duration_seconds": 15,
                "include_narration": True,
                "narration_speaker": "Emma",
                "reference_url": "https://videosync.video",
            },
        },
    ]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--env", default="video_sync.cloudrun.env.yaml")
    parser.add_argument("--base-url", default=DEFAULT_BASE_URL)
    parser.add_argument("--create", choices=["saas", "agency", "all"])
    parser.add_argument("--list", action="store_true")
    parser.add_argument("--workflows", action="store_true")
    parser.add_argument("--workflow-id")
    parser.add_argument("--events", action="store_true")
    parser.add_argument("--limit", type=int, default=10)
    args = parser.parse_args()

    config = parse_env_yaml(Path(args.env))
    token = mint_jwt(config, load_admin_user(config))

    if args.create:
        payloads = sample_payloads()
        selected = payloads if args.create == "all" else [payloads[0] if args.create == "saas" else payloads[1]]
        for payload in selected:
            create_delivery(args.base_url, token, payload)

    if args.list or not args.create:
        list_deliveries(args.base_url, token, args.limit)

    if args.workflows:
        query_workflows(config, args.workflow_id, args.limit)

    if args.events:
        if not args.workflow_id:
            raise RuntimeError("--events requires --workflow-id")
        query_workflow_events(config, args.workflow_id, args.limit)

    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(f"revenue smoke failed: {exc}", file=sys.stderr)
        raise SystemExit(1)
