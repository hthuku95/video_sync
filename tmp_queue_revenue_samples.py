#!/usr/bin/env python3
import base64
import hashlib
import hmac
import json
import os
import subprocess
import time
import urllib.error
import urllib.request
from pathlib import Path


BASE_URL = "https://video-sync-b7blplmlxq-uc.a.run.app"
RUN_LABEL = os.environ.get("REVENUE_SAMPLE_RUN_LABEL", "v1b")
SAMPLE_CLIENT_REFS = (
    f"portfolio:revenue-v1:{RUN_LABEL}:saas-demo:calcom",
    f"portfolio:revenue-v1:{RUN_LABEL}:agency-pack:framer-client",
    f"portfolio:revenue-v1:{RUN_LABEL}:education:vector-db",
)


def load_env() -> None:
    for raw in Path(".env").read_text().splitlines():
        line = raw.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        os.environ[key.strip()] = value.strip().strip('"').strip("'")


def b64url(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode("ascii")


def make_jwt(user: dict) -> str:
    secret = os.environ["JWT_SECRET"].encode("utf-8")
    now = int(time.time())
    header = {"alg": "HS256", "typ": "JWT"}
    payload = {
        "sub": str(user["id"]),
        "username": user["username"],
        "email": user["email"],
        "is_superuser": bool(user["is_superuser"]),
        "is_staff": bool(user["is_staff"]),
        "is_clipper": bool(user.get("is_clipper", False)),
        "exp": now + 24 * 60 * 60,
        "iat": now,
    }
    signing_input = f"{b64url(json.dumps(header, separators=(',', ':')).encode())}.{b64url(json.dumps(payload, separators=(',', ':')).encode())}"
    signature = hmac.new(secret, signing_input.encode("ascii"), hashlib.sha256).digest()
    return f"{signing_input}.{b64url(signature)}"


def admin_user() -> dict:
    sql = (
        "SELECT json_build_object('id', id, 'email', email, 'username', username, "
        "'is_superuser', is_superuser, 'is_staff', is_staff, 'is_clipper', is_clipper) "
        "FROM users WHERE is_active = true AND (is_superuser = true OR is_staff = true) "
        "ORDER BY is_superuser DESC, id LIMIT 1;"
    )
    result = subprocess.run(
        ["psql", os.environ["DATABASE_URL"], "-t", "-A", "-c", sql],
        check=True,
        text=True,
        capture_output=True,
    )
    line = result.stdout.strip().splitlines()[0]
    return json.loads(line)


def post_delivery(token: str, payload: dict) -> dict:
    req = urllib.request.Request(
        f"{BASE_URL}/api/admin/deliveries",
        data=json.dumps(payload).encode("utf-8"),
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
        },
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=60) as resp:
            return json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as error:
        body = error.read().decode("utf-8", "replace")
        raise RuntimeError(f"HTTP {error.code}: {body}") from error


def main() -> None:
    load_env()
    if len(os.sys.argv) > 1 and os.sys.argv[1] == "status":
        refs_sql = ",".join("'" + ref.replace("'", "''") + "'" for ref in SAMPLE_CLIENT_REFS)
        sql = """
        SELECT d.client_ref,
               d.id,
               d.title,
               d.status AS delivery_status,
               d.output_r2_url IS NOT NULL AS has_output,
               d.error_message,
               aw.status AS workflow_status,
               aw.current_step,
               aw.error_message AS workflow_error,
               aw.updated_at
          FROM deliveries d
          LEFT JOIN app_workflows aw ON aw.id = d.workflow_id
         WHERE d.client_ref = ANY(ARRAY[%s]::text[])
         ORDER BY d.created_at DESC;
        """ % refs_sql
        result = subprocess.run(
            ["psql", os.environ["DATABASE_URL"], "-c", sql],
            text=True,
            capture_output=True,
        )
        if result.returncode != 0:
            print("Status query failed:")
            print(result.stderr.replace(os.environ["DATABASE_URL"], "[DATABASE_URL]"))
            os.sys.exit(result.returncode)
        print(result.stdout)
        return

    if len(os.sys.argv) > 1 and os.sys.argv[1] == "events":
        refs_sql = ",".join("'" + ref.replace("'", "''") + "'" for ref in SAMPLE_CLIENT_REFS)
        sql = """
        WITH selected AS (
            SELECT d.workflow_id
              FROM deliveries d
             WHERE d.client_ref = ANY(ARRAY[%s]::text[])
        )
        SELECT e.workflow_id,
               e.event_type,
               e.node_name,
               left(e.message, 180) AS message,
               e.created_at
          FROM app_workflow_events e
          JOIN selected s ON s.workflow_id = e.workflow_id
         ORDER BY e.created_at DESC
         LIMIT 30;
        """ % refs_sql
        result = subprocess.run(
            ["psql", os.environ["DATABASE_URL"], "-c", sql],
            text=True,
            capture_output=True,
        )
        if result.returncode != 0:
            print("Event query failed:")
            print(result.stderr.replace(os.environ["DATABASE_URL"], "[DATABASE_URL]"))
            os.sys.exit(result.returncode)
        print(result.stdout)
        return

    user = admin_user()
    token = make_jwt(user)
    samples = [
        {
            "client_ref": SAMPLE_CLIENT_REFS[0],
            "title": "Speculative SaaS Demo - Cal.com Scheduling Workflow",
            "gig_type": "long_form_video",
            "prompt": (
                "Create a polished buyer-facing SaaS/app demo video for Cal.com. "
                "Use https://cal.com/ as the reference. Explain the scheduling pain, show the product promise, "
                "highlight team scheduling, booking pages, integrations, and end with a strong CTA. "
                "Make this look like a $499 launch/demo pack sample with narration, captions, motion, and QA."
            ),
            "style": "modern product launch",
            "duration": 75,
            "extra": {
                "service_offer": "saas_launch_pack",
                "offer_type": "saas_launch_pack",
                "source_url": "https://cal.com/",
                "reference_url": "https://cal.com/",
                "include_narration": True,
                "narration_speaker": "Emma",
                "segment_duration_seconds": 25,
                "portfolio_category": "saas_demo",
                "sales_positioning": "$499 full demo pack sample",
            },
        },
        {
            "client_ref": SAMPLE_CLIENT_REFS[1],
            "title": "Speculative Agency Website-to-Video Sample - Framer Client Launch",
            "gig_type": "long_form_video",
            "prompt": (
                "Create a polished website-to-video agency sample for a Framer/Webflow agency prospect. "
                "Use https://www.framer.com/ as the reference URL and demonstrate how an agency could turn a client website "
                "into a launch video. Include a clear agency-resell angle: send three client websites, receive three "
                "client-ready videos. Make it suitable for a $999 agency pack conversation."
            ),
            "style": "premium agency motion",
            "duration": 70,
            "extra": {
                "service_offer": "creator_manager_fulfillment",
                "offer_type": "agency_website_to_video_pack",
                "source_url": "https://www.framer.com/",
                "reference_url": "https://www.framer.com/",
                "include_narration": True,
                "narration_speaker": "Emma",
                "segment_duration_seconds": 25,
                "portfolio_category": "agency_pack",
                "sales_positioning": "$999 for 3 client videos agency pack sample",
            },
        },
        {
            "client_ref": SAMPLE_CLIENT_REFS[2],
            "title": "Speculative Education Explainer - Vector Databases for SaaS Founders",
            "gig_type": "long_form_video",
            "prompt": (
                "Create an education-style explainer video that teaches SaaS founders what vector databases are, "
                "why embeddings matter, and how semantic search improves product UX. Use Manim/LaTeX-style visuals "
                "where useful, simple diagrams, narration, captions, and a clear CTA for ordering custom explainers."
            ),
            "style": "clean educational motion",
            "duration": 80,
            "extra": {
                "service_offer": "education_explainer_pack",
                "offer_type": "education_explainer_pack",
                "include_narration": True,
                "narration_speaker": "Emma",
                "segment_duration_seconds": 25,
                "portfolio_category": "education",
                "sales_positioning": "Education/Manim/LaTeX explainer sample",
            },
        },
    ]

    created = []
    for sample in samples:
        response = post_delivery(token, sample)
        created.append(
            {
                "title": sample["title"],
                "delivery_id": response.get("delivery_id"),
                "workflow_id": response.get("workflow_id"),
                "delivery_url": f"{BASE_URL}/delivery/{response.get('delivery_id')}",
            }
        )
    print(json.dumps(created, indent=2))


if __name__ == "__main__":
    main()
