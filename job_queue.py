"""
Async Job Queue — Phase 7 (SQS + DynamoDB)

Job dispatch uses SQS (Standard queue) so workers can run on any EC2 instance.
Job status is stored in DynamoDB for fast, scalable reads without connection
pooling. Progress events remain in PostgreSQL (via progress_store.py) for UI.

Flow:
  submit()  -> DDB PutItem (PENDING) -> SQS SendMessage
  worker()  -> SQS ReceiveMessage -> DDB UpdateItem (RUNNING)
              -> execute handler -> DDB UpdateItem (COMPLETED|FAILED)
              -> SQS DeleteMessage
  get()     -> DDB GetItem
  startup   -> DDB Scan for PENDING/RUNNING -> mark RECOVERED (SQS redelivers)

Usage (from server.py):
    from tools.job_queue import queue, JobStatus

    job_id = await queue.submit("manim_execute_script", {"description": "..."})
    status = queue.get(job_id)

REST endpoints (wired in server.py):
    POST /api/jobs           -- submit a job, returns {"job_id": str}
    GET  /api/jobs/{job_id}  -- poll job status
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

import boto3

from tools.progress_store import record_job_progress


# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

SQS_QUEUE_URL = os.getenv(
    "SQS_QUEUE_URL",
    "https://sqs.us-east-1.amazonaws.com/405837966164/blender-mcp-jobs",
)
DYNAMODB_TABLE = os.getenv("DYNAMODB_TABLE", "blender-mcp-jobs")
AWS_REGION = os.getenv("AWS_REGION", "us-east-1")


# ---------------------------------------------------------------------------
# AWS clients
# ---------------------------------------------------------------------------

_sqs = boto3.client("sqs", region_name=AWS_REGION)
_ddb = boto3.resource("dynamodb", region_name=AWS_REGION)
_table = _ddb.Table(DYNAMODB_TABLE)

_loop: asyncio.AbstractEventLoop | None = None


def _get_loop() -> asyncio.AbstractEventLoop:
    global _loop
    if _loop is None:
        try:
            _loop = asyncio.get_running_loop()
        except RuntimeError:
            _loop = asyncio.new_event_loop()
    return _loop


# ---------------------------------------------------------------------------
# Async wrappers for synchronous boto3 calls
# ---------------------------------------------------------------------------

async def _async_put_status(status: JobStatus) -> None:
    loop = _get_loop()
    await loop.run_in_executor(None, _put_status_sync, status)


async def _async_get_status(job_id: str) -> JobStatus | None:
    loop = _get_loop()
    return await loop.run_in_executor(None, _get_status_sync, job_id)


async def _async_sqs_send_message(body: dict) -> None:
    loop = _get_loop()
    await loop.run_in_executor(None, lambda: _sqs.send_message(
        QueueUrl=SQS_QUEUE_URL,
        MessageBody=json.dumps(body),
    ))


async def _async_sqs_receive_message() -> dict:
    loop = _get_loop()
    return await loop.run_in_executor(None, lambda: _sqs.receive_message(
        QueueUrl=SQS_QUEUE_URL,
        MaxNumberOfMessages=1,
        WaitTimeSeconds=20,
        AttributeNames=["All"],
        MessageAttributeNames=["All"],
    ))


async def _async_sqs_delete_message(receipt_handle: str) -> None:
    loop = _get_loop()
    await loop.run_in_executor(None, lambda: _sqs.delete_message(
        QueueUrl=SQS_QUEUE_URL,
        ReceiptHandle=receipt_handle,
    ))


async def _async_recover_orphans() -> list[str]:
    loop = _get_loop()
    return await loop.run_in_executor(None, _recover_orphans_ddb_sync)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

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
# DynamoDB I/O (synchronous implementations)
# ---------------------------------------------------------------------------

def _ddb_to_status(item: dict) -> JobStatus:
    return JobStatus(
        job_id=item["job_id"],
        tool=item.get("tool", ""),
        workflow_thread_id=item.get("workflow_thread_id", ""),
        state=State(item.get("state", "pending")),
        result=item.get("result"),
        error=item.get("error", ""),
        args=item.get("args"),
        created_at=item.get("created_at", _now()),
        started_at=item.get("started_at", ""),
        finished_at=item.get("finished_at", ""),
    )


def _put_status_sync(status: JobStatus) -> None:
    item = {
        "job_id": status.job_id,
        "tool": status.tool,
        "workflow_thread_id": status.workflow_thread_id,
        "state": status.state.value,
        "created_at": status.created_at,
        "started_at": status.started_at,
        "finished_at": status.finished_at,
        "error": status.error,
    }
    if status.result is not None:
        item["result"] = status.result
    if status.args is not None:
        item["args"] = status.args
    _table.put_item(Item=item)


def _get_status_sync(job_id: str) -> JobStatus | None:
    r = _table.get_item(Key={"job_id": job_id})
    item = r.get("Item")
    return _ddb_to_status(item) if item else None


def _recover_orphans_ddb_sync() -> list[str]:
    orphans: list[str] = []
    response = _table.scan(
        FilterExpression=boto3.dynamodb.conditions.Attr("state").is_in(["pending", "running"]),
    )
    for item in response.get("Items", []):
        job_id = item["job_id"]
        orphans.append(job_id)
        _table.update_item(
            Key={"job_id": job_id},
            UpdateExpression="SET #st = :recovered, #err = :msg",
            ExpressionAttributeNames={"#st": "state", "#err": "error"},
            ExpressionAttributeValues={":recovered": State.RECOVERED.value, ":msg": "lost on restart"},
        )

    while "LastEvaluatedKey" in response:
        response = _table.scan(
            FilterExpression=boto3.dynamodb.conditions.Attr("state").is_in(["pending", "running"]),
            ExclusiveStartKey=response["LastEvaluatedKey"],
        )
        for item in response.get("Items", []):
            job_id = item["job_id"]
            orphans.append(job_id)
            _table.update_item(
                Key={"job_id": job_id},
                UpdateExpression="SET #st = :recovered, #err = :msg",
                ExpressionAttributeNames={"#st": "state", "#err": "error"},
                ExpressionAttributeValues={":recovered": State.RECOVERED.value, ":msg": "lost on restart"},
            )
    return orphans


def _list_from_ddb_sync(limit: int = 100) -> list[dict]:
    response = _table.scan(Limit=limit)
    items = response.get("Items", [])
    items.sort(key=lambda i: i.get("created_at", ""), reverse=True)
    result = []
    for item in items[:limit]:
        result.append(_ddb_to_status(item).to_dict())
    while len(result) < limit and "LastEvaluatedKey" in response:
        response = _table.scan(
            Limit=limit - len(result),
            ExclusiveStartKey=response["LastEvaluatedKey"],
        )
        for item in response.get("Items", []):
            result.append(_ddb_to_status(item).to_dict())
        items.sort(key=lambda i: i.get("created_at", ""), reverse=True)
    return result[:limit]


# ---------------------------------------------------------------------------
# Queue
# ---------------------------------------------------------------------------

class JobQueue:
    """
    SQS-dispatched, DynamoDB-backed async job queue.

    Workers run as asyncio tasks that long-poll SQS.  Job status lives in
    DynamoDB so it survives restarts and is accessible from any worker.
    Orphaned PENDING/RUNNING jobs from a prior lifecycle are marked RECOVERED;
    the corresponding SQS messages will be redelivered after the visibility
    timeout expires.
    """

    def __init__(self, max_workers: int = 3):
        self._max_workers = max_workers
        self._tool_registry: dict[str, Callable[..., Coroutine[Any, Any, dict]]] = {}
        self._started = False
        self._recovered: list[str] = []
        self._cancel_set: set[str] = set()
        self._shutdown = False

    def register(self, tool_name: str, fn: Callable[..., Coroutine[Any, Any, dict]]) -> None:
        """Register a coroutine function as the handler for a tool name."""
        self._tool_registry[tool_name] = fn

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    async def submit(self, tool_name: str, args: dict) -> str:
        """Submit a job and return its job_id.  Starts workers on first call."""
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
        await _async_put_status(status)

        await record_job_progress(
            job_id=job_id,
            workflow_thread_id=workflow_thread_id,
            tool=tool_name,
            state=State.PENDING.value,
            stage="queued",
            message=f"Queued {tool_name} job",
            details={"arg_keys": sorted(normalized_args.keys())},
        )

        await _async_sqs_send_message({
            "job_id": job_id,
            "tool": tool_name,
            "args": normalized_args,
        })
        return job_id

    def get(self, job_id: str) -> JobStatus | None:
        """Read job status from DynamoDB."""
        return _get_status_sync(job_id)

    def cancel(self, job_id: str) -> bool:
        """Cancel a pending or running job. Returns True if cancelled."""
        status = _get_status_sync(job_id)
        if status is None:
            return False
        if status.state in (State.COMPLETED, State.FAILED, State.CANCELLED, State.RECOVERED):
            return False
        status.state = State.CANCELLED
        status.finished_at = _now()
        _put_status_sync(status)
        self._cancel_set.add(job_id)
        return True

    def is_cancelled(self, job_id: str) -> bool:
        """Check if a job has been cancelled (checks local set first for speed)."""
        if job_id in self._cancel_set:
            return True
        status = _get_status_sync(job_id)
        if status and status.state == State.CANCELLED:
            self._cancel_set.add(job_id)
            return True
        return False

    def list_jobs(self, limit: int = 100) -> list[dict]:
        """Return the most recent jobs from DynamoDB."""
        return _list_from_ddb_sync(limit)

    def recovered_jobs(self) -> list[str]:
        """Return job IDs that were recovered (lost on restart)."""
        return list(self._recovered)

    # ------------------------------------------------------------------
    # Worker loop
    # ------------------------------------------------------------------

    async def _start_workers(self) -> None:
        self._recovered = await _async_recover_orphans()
        self._started = True
        for _ in range(self._max_workers):
            asyncio.create_task(self._worker())

    async def _worker(self) -> None:
        """Long-poll SQS for new jobs.  Runs forever."""
        while not self._shutdown:
            try:
                response = await _async_sqs_receive_message()
            except Exception:
                await asyncio.sleep(5)
                continue

            messages = response.get("Messages", [])
            if not messages:
                continue

            msg = messages[0]
            receipt_handle = msg["ReceiptHandle"]

            try:
                body = json.loads(msg["Body"])
            except (json.JSONDecodeError, KeyError):
                await _async_sqs_delete_message(receipt_handle)
                continue

            job_id = body.get("job_id", "")
            if not job_id:
                await _async_sqs_delete_message(receipt_handle)
                continue

            await self._handle_job(job_id, receipt_handle)

    async def _handle_job(self, job_id: str, receipt_handle: str) -> None:
        status = await _async_get_status(job_id)
        if status is None:
            await _async_sqs_delete_message(receipt_handle)
            return

        args = status.args or {}
        handler = self._tool_registry.get(status.tool)

        status.state = State.RUNNING
        status.started_at = _now()
        status.result = None
        await _async_put_status(status)

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

        _JOB_TIMEOUT = int(os.getenv("JOB_TIMEOUT_SECS", "1500"))

        try:
            if handler is None:
                raise ValueError(f"No handler registered for tool '{status.tool}'")

            if self.is_cancelled(job_id):
                status.state = State.CANCELLED
                status.finished_at = _now()
                _put_status_sync(status)
                asyncio.create_task(_record_cancelled_progress_async(status))
                await _async_sqs_delete_message(receipt_handle)
                return

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
            await _async_put_status(status)

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
            _put_status_sync(status)
            asyncio.create_task(_record_failure_progress_async(status))

        except Exception as exc:
            status.state = State.FAILED
            status.error = str(exc)
            _put_status_sync(status)
            asyncio.create_task(_record_failure_progress_async(status))

        finally:
            try:
                await _async_sqs_delete_message(receipt_handle)
            except Exception:
                pass

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
# Helpers (standalone to avoid closure issues in exception handlers)
# ---------------------------------------------------------------------------

async def _record_failure_progress_async(status: JobStatus) -> None:
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


async def _record_cancelled_progress_async(status: JobStatus) -> None:
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


# ---------------------------------------------------------------------------
# Singleton  (max_workers from env, default 3)
# ---------------------------------------------------------------------------

queue = JobQueue(max_workers=int(os.getenv("JOB_QUEUE_WORKERS", "3")))
