"""
Async Job Queue — Phase 6 (PostgreSQL-backed)

Job status is stored in PostgreSQL (Neon) so it survives service restarts.
Workers still run in-process via asyncio; the internal dispatch queue is lost
on restart but status is recovered from the DB.  On init, any PENDING or
RUNNING jobs from a previous lifecycle are marked as FAILED (RECOVERED) since
their handler future was lost.

Usage (from server.py):
    from tools.job_queue import queue, JobStatus

    job_id = await queue.submit("manim_execute_script", {"description": "..."})
    status = queue.get(job_id)   # reads from PostgreSQL

REST endpoints (wired in server.py):
    POST /api/jobs           — submit a job, returns {"job_id": str}
    GET  /api/jobs/{job_id}  — poll job status
"""

from __future__ import annotations

import asyncio
import inspect
import json
import os
import uuid
from dataclasses import dataclass, field
from datetime import datetime, timezone
from enum import Enum
from typing import Any, Callable, Coroutine

from tools.progress_store import record_job_progress


# ---------------------------------------------------------------------------
# DB helpers
# ---------------------------------------------------------------------------

def _get_db_url() -> str:
    return os.environ.get(
        "DATABASE_URL",
        "postgresql://neondb_owner:npg_0q3eLrfUoRaH@ep-proud-mode-aevhwm20-pooler.c-2.us-east-2.aws.neon.tech/neondb?sslmode=require&channel_binding=require",
    )


def _now() -> str:
    return datetime.now(timezone.utc).isoformat()


def _to_json(val: Any) -> str:
    return json.dumps(val, default=str)


def _from_json(val: str | None) -> Any:
    if val is None:
        return None
    try:
        return json.loads(val)
    except (json.JSONDecodeError, TypeError):
        return val


class State(str, Enum):
    PENDING   = "pending"
    RUNNING   = "running"
    COMPLETED = "completed"
    FAILED    = "failed"
    CANCELLED = "cancelled"
    RECOVERED = "recovered"


@dataclass
class JobStatus:
    job_id:    str
    tool:      str
    workflow_thread_id: str       = ""
    state:     State              = State.PENDING
    result:    dict | None        = None
    error:     str                = ""
    args:      dict | None        = None
    created_at: str               = field(default_factory=_now)
    started_at: str               = ""
    finished_at: str              = ""

    def to_dict(self) -> dict:
        return {
            "job_id":      self.job_id,
            "tool":        self.tool,
            "workflow_thread_id": self.workflow_thread_id,
            "state":       self.state.value,
            "result":      self.result,
            "error":       self.error,
            "created_at":  self.created_at,
            "started_at":  self.started_at,
            "finished_at": self.finished_at,
        }


# ---------------------------------------------------------------------------
# Queue
# ---------------------------------------------------------------------------

class JobQueue:
    """
    PostgreSQL-backed async job queue.

    Job status is stored permanently in the `job_status` table.  Workers run
    concurrently up to ``max_workers``.  On startup, any PENDING/RUNNING jobs
    from a previous lifecycle are marked as RECOVERED (they can't be resumed
    because the in-process handler future was lost).
    """

    def __init__(self, max_workers: int = 3):
        self._db_url: str = _get_db_url()
        self._pending: asyncio.Queue[str] = asyncio.Queue()
        self._max_workers = max_workers
        self._tool_registry: dict[str, Callable[..., Coroutine[Any, Any, dict]]] = {}
        self._started = False
        self._recovered: list[str] = []

    def register(self, tool_name: str, fn: Callable[..., Coroutine[Any, Any, dict]]) -> None:
        """Register a coroutine function as the handler for a tool name."""
        self._tool_registry[tool_name] = fn

    # ------------------------------------------------------------------
    # DB I/O
    # ------------------------------------------------------------------

    def _conn(self):
        import psycopg
        return psycopg.connect(self._db_url)

    def _upsert_status(self, status: JobStatus) -> None:
        """Write job status to PostgreSQL (INSERT … ON CONFLICT UPDATE)."""
        with self._conn() as conn:
            conn.execute(
                """
                INSERT INTO job_status
                    (job_id, tool, workflow_thread_id, state, result, error, args,
                     created_at, started_at, finished_at)
                VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s, %s)
                ON CONFLICT (job_id) DO UPDATE SET
                    state        = EXCLUDED.state,
                    result       = EXCLUDED.result,
                    error        = EXCLUDED.error,
                    started_at   = COALESCE(EXCLUDED.started_at, job_status.started_at),
                    finished_at  = EXCLUDED.finished_at
                """,
                (
                    status.job_id,
                    status.tool,
                    status.workflow_thread_id,
                    status.state.value,
                    _to_json(status.result) if status.result is not None else None,
                    status.error,
                    _to_json(status.args) if status.args is not None else None,
                    status.created_at,
                    status.started_at or None,
                    status.finished_at or None,
                ),
            )
            conn.commit()

    def _read_status(self, job_id: str) -> JobStatus | None:
        """Read a single job from PostgreSQL."""
        with self._conn() as conn:
            row = conn.execute(
                "SELECT job_id, tool, workflow_thread_id, state, result, error, args, "
                "created_at, started_at, finished_at FROM job_status WHERE job_id = %s",
                (job_id,),
            ).fetchone()
        if row is None:
            return None
        return JobStatus(
            job_id=row[0], tool=row[1], workflow_thread_id=row[2] or "",
            state=State(row[3]),
            result=_from_json(row[4]),
            error=row[5] or "",
            args=_from_json(row[6]),
            created_at=row[7].isoformat() if row[7] else _now(),
            started_at=row[8].isoformat() if row[8] else "",
            finished_at=row[9].isoformat() if row[9] else "",
        )

    def _recover_orphans(self) -> None:
        """Mark any PENDING or RUNNING jobs from a prior lifecycle as RECOVERED."""
        with self._conn() as conn:
            orphans = conn.execute(
                "SELECT job_id FROM job_status WHERE state IN ('pending', 'running')"
            ).fetchall()
            if orphans:
                ids = [r[0] for r in orphans]
                conn.execute(
                    "UPDATE job_status SET state = 'recovered', error = 'lost on restart' "
                    "WHERE state IN ('pending', 'running')"
                )
                conn.commit()
                self._recovered = ids

    def _list_from_db(self, limit: int = 100) -> list[dict]:
        """Return the most recent ``limit`` jobs from PostgreSQL."""
        with self._conn() as conn:
            rows = conn.execute(
                "SELECT job_id, tool, workflow_thread_id, state, result, error, args, "
                "created_at, started_at, finished_at "
                "FROM job_status ORDER BY created_at DESC LIMIT %s",
                (limit,),
            ).fetchall()
        result = []
        for row in rows:
            js = JobStatus(
                job_id=row[0], tool=row[1], workflow_thread_id=row[2] or "",
                state=State(row[3]),
                result=_from_json(row[4]),
                error=row[5] or "",
                args=_from_json(row[6]),
                created_at=row[7].isoformat() if row[7] else _now(),
                started_at=row[8].isoformat() if row[8] else "",
                finished_at=row[9].isoformat() if row[9] else "",
            )
            result.append(js.to_dict())
        return result

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    async def submit(self, tool_name: str, args: dict) -> str:
        """Submit a job and return its job_id.  Starts worker loop on first call."""
        if not self._started:
            await self._start_workers()

        job_id = str(uuid.uuid4())
        normalized_args = dict(args or {})
        workflow_thread_id = str(normalized_args.get("workflow_thread_id") or job_id)
        normalized_args["workflow_thread_id"] = workflow_thread_id

        status = JobStatus(
            job_id=job_id,
            tool=tool_name,
            workflow_thread_id=workflow_thread_id,
            args=normalized_args,
        )
        self._upsert_status(status)

        await record_job_progress(
            job_id=job_id,
            workflow_thread_id=workflow_thread_id,
            tool=tool_name,
            state=State.PENDING.value,
            stage="queued",
            message=f"Queued {tool_name} job",
            details={"arg_keys": sorted(normalized_args.keys())},
        )
        await self._pending.put(job_id)
        return job_id

    def get(self, job_id: str) -> JobStatus | None:
        """Read job status from PostgreSQL."""
        return self._read_status(job_id)

    def cancel(self, job_id: str) -> bool:
        """Cancel a pending or running job. Returns True if cancelled."""
        status = self._read_status(job_id)
        if status is None:
            return False
        if status.state in (State.COMPLETED, State.FAILED, State.CANCELLED, State.RECOVERED):
            return False
        status.state = State.CANCELLED
        status.finished_at = _now()
        self._upsert_status(status)
        return True

    def is_cancelled(self, job_id: str) -> bool:
        """Check if a job has been cancelled."""
        status = self._read_status(job_id)
        if status is None:
            return False
        return status.state == State.CANCELLED

    def list_jobs(self, limit: int = 100) -> list[dict]:
        """Return the most recent jobs from PostgreSQL."""
        return self._list_from_db(limit)

    def recovered_jobs(self) -> list[str]:
        """Return job IDs that were recovered (lost on restart)."""
        return list(self._recovered)

    # ------------------------------------------------------------------
    # Worker loop
    # ------------------------------------------------------------------

    async def _start_workers(self) -> None:
        # Recover orphans before accepting new jobs
        self._recover_orphans()
        self._started = True
        for _ in range(self._max_workers):
            asyncio.create_task(self._worker())

    async def _worker(self) -> None:
        while True:
            job_id = await self._pending.get()
            status = self._read_status(job_id)
            if status is None:
                self._pending.task_done()
                continue

            args = status.args or {}
            handler = self._tool_registry.get(status.tool)

            status.state = State.RUNNING
            status.started_at = _now()
            status.result = None
            self._upsert_status(status)

            await record_job_progress(
                job_id=status.job_id,
                workflow_thread_id=status.workflow_thread_id or status.job_id,
                tool=status.tool,
                state=State.RUNNING.value,
                stage="dispatch",
                message=f"Dispatching {status.tool} handler",
                details={},
                started_at=datetime.now(timezone.utc),
            )

            if handler is None:
                status.state = State.FAILED
                status.error = f"No handler registered for tool '{status.tool}'"
                status.finished_at = _now()
                self._save_final_status(status)
                self._pending.task_done()
                continue

            if self.is_cancelled(job_id):
                status.state = State.CANCELLED
                status.finished_at = _now()
                self._save_cancelled_status(status)
                self._pending.task_done()
                continue

            _JOB_TIMEOUT = int(os.getenv("JOB_TIMEOUT_SECS", "1500"))
            try:
                handler_args = self._filter_handler_args(handler, args)
                result = await asyncio.wait_for(
                    handler(**handler_args), timeout=_JOB_TIMEOUT
                )

                if self.is_cancelled(job_id):
                    status.state = State.CANCELLED
                    status.result = result
                else:
                    status.state = State.COMPLETED
                    status.result = result
                status.finished_at = _now()
                self._upsert_status(status)
                stage = "cancelled" if status.state == State.CANCELLED else "completed"
                await record_job_progress(
                    job_id=status.job_id,
                    workflow_thread_id=status.workflow_thread_id or status.job_id,
                    tool=status.tool,
                    state=status.state.value,
                    stage=stage,
                    message=f"{status.tool} job {stage}",
                    details={},
                    result=result,
                    finished_at=datetime.now(timezone.utc),
                )

            except asyncio.TimeoutError:
                status.state = State.FAILED
                status.error = f"Job exceeded maximum runtime of {_JOB_TIMEOUT}s"
                self._save_final_status(status)

            except Exception as exc:
                status.state = State.FAILED
                status.error = str(exc)
                self._save_final_status(status)

            finally:
                self._pending.task_done()

    def _save_final_status(self, status: JobStatus) -> None:
        status.finished_at = _now()
        self._upsert_status(status)
        asyncio.create_task(self._record_failure_progress(status))

    def _save_cancelled_status(self, status: JobStatus) -> None:
        status.finished_at = _now()
        self._upsert_status(status)
        asyncio.create_task(self._record_cancelled_progress(status))

    async def _record_failure_progress(self, status: JobStatus) -> None:
        await record_job_progress(
            job_id=status.job_id,
            workflow_thread_id=status.workflow_thread_id or status.job_id,
            tool=status.tool,
            state=status.state.value,
            stage="failed",
            message=status.error,
            details={"exception_type": "error"},
            error=status.error,
            finished_at=datetime.now(timezone.utc),
        )

    async def _record_cancelled_progress(self, status: JobStatus) -> None:
        await record_job_progress(
            job_id=status.job_id,
            workflow_thread_id=status.workflow_thread_id or status.job_id,
            tool=status.tool,
            state=State.CANCELLED.value,
            stage="cancelled",
            message="Job was cancelled",
            details={},
            finished_at=datetime.now(timezone.utc),
        )

    @staticmethod
    def _filter_handler_args(
        handler: Callable[..., Coroutine[Any, Any, dict]],
        args: dict,
    ) -> dict:
        try:
            signature = inspect.signature(handler)
        except (TypeError, ValueError):
            return dict(args)

        if any(
            parameter.kind == inspect.Parameter.VAR_KEYWORD
            for parameter in signature.parameters.values()
        ):
            return dict(args)

        allowed = set(signature.parameters)
        return {key: value for key, value in args.items() if key in allowed}


# ---------------------------------------------------------------------------
# Singleton  (max_workers from env, default 3)
# ---------------------------------------------------------------------------

queue = JobQueue(max_workers=int(os.getenv("JOB_QUEUE_WORKERS", "3")))
