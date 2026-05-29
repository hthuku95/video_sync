#!/usr/bin/env python3
import os
import subprocess
from pathlib import Path


def load_env() -> None:
    for raw in Path(".env").read_text().splitlines():
        line = raw.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        os.environ[key.strip()] = value.strip().strip('"').strip("'")


def main() -> None:
    load_env()
    sql = """
    WITH fixed AS (
        UPDATE deliveries
           SET status = 'completed',
               error_message = NULL,
               completed_at = COALESCE(completed_at, NOW())
         WHERE client_ref LIKE 'portfolio:revenue-v1:v1c:%'
           AND output_r2_url IS NOT NULL
        RETURNING id, workflow_id, client_ref, output_r2_url
    ),
    fixed_workflows AS (
        UPDATE app_workflows aw
           SET status = 'completed',
               current_step = 'completed',
               error_message = NULL,
               completed_at = COALESCE(completed_at, NOW()),
               updated_at = NOW()
          FROM fixed
         WHERE aw.id = fixed.workflow_id
        RETURNING aw.id
    )
    SELECT client_ref, id, output_r2_url FROM fixed ORDER BY client_ref;
    """
    result = subprocess.run(
        ["psql", os.environ["DATABASE_URL"], "-c", sql],
        text=True,
        capture_output=True,
    )
    if result.returncode != 0:
        print(result.stderr.replace(os.environ["DATABASE_URL"], "[DATABASE_URL]"))
        raise SystemExit(result.returncode)
    print(result.stdout)


if __name__ == "__main__":
    main()
