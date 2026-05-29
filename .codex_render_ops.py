import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path


def load_env(path=".env"):
    env = {}
    p = Path(path)
    if not p.exists():
        return env
    for line in p.read_text(errors="ignore").splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        env[key.strip()] = value.strip().strip('"').strip("'")
    return env


ENV = {**os.environ, **load_env()}


def request(method, url, token=None, payload=None, headers=None):
    merged = {"Accept": "application/json"}
    if token:
        merged["Authorization"] = f"Bearer {token}"
    if headers:
        merged.update(headers)
    data = None
    if payload is not None:
        data = json.dumps(payload).encode()
        merged["Content-Type"] = "application/json"
    req = urllib.request.Request(url, data=data, method=method, headers=merged)
    try:
        with urllib.request.urlopen(req, timeout=90) as resp:
            body = resp.read().decode()
            return json.loads(body) if body else {}
    except urllib.error.HTTPError as exc:
        detail = exc.read().decode(errors="replace")
        raise RuntimeError(f"{method} {url} failed: {exc.code} {detail}") from exc


def render_services():
    key = ENV.get("RENDER_API_KEY")
    if not key:
        raise SystemExit("RENDER_API_KEY not set")
    data = request("GET", "https://api.render.com/v1/services?limit=100", key)
    for item in data:
        svc = item.get("service", item)
        print(json.dumps({
            "id": svc.get("id"),
            "name": svc.get("name"),
            "type": svc.get("type"),
            "repo": svc.get("repo"),
            "branch": svc.get("branch"),
            "url": (svc.get("serviceDetails") or {}).get("url"),
        }))


def render_deploys(service_id):
    key = ENV.get("RENDER_API_KEY")
    data = request("GET", f"https://api.render.com/v1/services/{service_id}/deploys?limit=10", key)
    for item in data:
        deploy = item.get("deploy", item)
        commit = deploy.get("commit") or {}
        print(json.dumps({
            "id": deploy.get("id"),
            "status": deploy.get("status"),
            "createdAt": deploy.get("createdAt"),
            "updatedAt": deploy.get("updatedAt"),
            "commitId": commit.get("id") or deploy.get("commitId"),
            "commitMessage": commit.get("message") or deploy.get("commitMessage"),
        }))


def trigger_portfolio(base_url):
    email = ENV.get("TEST_ADMIN_EMAIL")
    password = ENV.get("TEST_ADMIN_PASSWORD")
    if not email or not password:
        raise SystemExit("TEST_ADMIN_EMAIL/TEST_ADMIN_PASSWORD not set")
    base_url = base_url.rstrip("/")
    login_payload = {"email": email, "password": password}
    login = request(
        "POST",
        f"{base_url}/api/auth/login",
        payload=login_payload,
        headers={"Accept": "application/json"},
    )
    token = login.get("token") or login.get("access_token") or login.get("jwt")
    if not token and isinstance(login.get("data"), dict):
        token = login["data"].get("token") or login["data"].get("access_token")
    if not token:
        raise SystemExit(f"Login succeeded but no token field found: {sorted(login.keys())}")
    result = request(
        "POST",
        f"{base_url}/api/admin/portfolio-samples/crypto-saas",
        token=token,
        payload={},
    )
    print(json.dumps(result, indent=2))


def list_portfolio(base_url):
    email = ENV.get("TEST_ADMIN_EMAIL")
    password = ENV.get("TEST_ADMIN_PASSWORD")
    if not email or not password:
        raise SystemExit("TEST_ADMIN_EMAIL/TEST_ADMIN_PASSWORD not set")
    base_url = base_url.rstrip("/")
    login = request(
        "POST",
        f"{base_url}/api/auth/login",
        payload={"email": email, "password": password},
        headers={"Accept": "application/json"},
    )
    token = login.get("token") or login.get("access_token") or login.get("jwt")
    if not token and isinstance(login.get("data"), dict):
        token = login["data"].get("token") or login["data"].get("access_token")
    if not token:
        raise SystemExit(f"Login succeeded but no token field found: {sorted(login.keys())}")
    result = request("GET", f"{base_url}/api/admin/portfolio-samples", token=token)
    print(json.dumps(result, indent=2))


def main():
    if len(sys.argv) < 2:
        raise SystemExit("usage: .codex_render_ops.py services|deploys SERVICE_ID|trigger BASE_URL")
    cmd = sys.argv[1]
    if cmd == "services":
        render_services()
    elif cmd == "deploys":
        render_deploys(sys.argv[2])
    elif cmd == "trigger":
        trigger_portfolio(sys.argv[2])
    elif cmd == "list":
        list_portfolio(sys.argv[2])
    else:
        raise SystemExit(f"unknown command: {cmd}")


if __name__ == "__main__":
    main()
