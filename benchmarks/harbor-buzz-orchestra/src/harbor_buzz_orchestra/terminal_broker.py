"""Serialized, trial-scoped bridge from agent MCP tools to Harbor execution."""

from __future__ import annotations

import asyncio
import json
import os
import time
from contextlib import suppress
from datetime import UTC, datetime
from pathlib import Path
from typing import Any
from uuid import uuid4

from harbor.environments.base import BaseEnvironment


class TerminalBrokerError(RuntimeError):
    """Raised when a terminal broker request is malformed or cannot be served."""


class SerializedTerminalBroker:
    """Serve terminal requests over a Unix socket, one execution at a time."""

    def __init__(
        self,
        *,
        environment: BaseEnvironment,
        socket_path: Path,
        log_path: Path,
    ) -> None:
        self._environment = environment
        self.socket_path = socket_path
        self.log_path = log_path
        self._execution_lock = asyncio.Lock()
        self._log_lock = asyncio.Lock()
        self._server: asyncio.AbstractServer | None = None

    async def __aenter__(self) -> SerializedTerminalBroker:
        await self.start()
        return self

    async def __aexit__(self, *_: object) -> None:
        await self.close()

    async def start(self) -> None:
        if self._server is not None:
            raise RuntimeError("terminal broker is already started")
        self.socket_path.parent.mkdir(parents=True, exist_ok=True)
        self.log_path.parent.mkdir(parents=True, exist_ok=True)
        with suppress(FileNotFoundError):
            self.socket_path.unlink()
        self._server = await asyncio.start_unix_server(
            self._serve_connection, path=self.socket_path
        )
        os.chmod(self.socket_path, 0o600)

    async def close(self) -> None:
        if self._server is not None:
            self._server.close()
            await self._server.wait_closed()
            self._server = None
        with suppress(FileNotFoundError):
            self.socket_path.unlink()

    async def _serve_connection(
        self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter
    ) -> None:
        try:
            raw = await reader.readline()
            if not raw:
                return
            try:
                request = json.loads(raw)
                response = await self.execute(request)
            except Exception as error:  # response boundary: never strand MCP client
                response = {"ok": False, "error": str(error)}
            writer.write(json.dumps(response, ensure_ascii=False).encode() + b"\n")
            await writer.drain()
        finally:
            writer.close()
            with suppress(Exception):
                await writer.wait_closed()

    async def execute(self, request: dict[str, Any]) -> dict[str, Any]:
        command = request.get("command")
        agent_id = request.get("agent_id")
        if not isinstance(command, str) or not command:
            raise TerminalBrokerError("command must be a non-empty string")
        if not isinstance(agent_id, str) or not agent_id:
            raise TerminalBrokerError("agent_id must be a non-empty string")

        cwd = self._optional_str(request, "cwd")
        timeout_sec = request.get("timeout_sec")
        if timeout_sec is not None and (
            isinstance(timeout_sec, bool)
            or not isinstance(timeout_sec, int)
            or timeout_sec < 1
        ):
            raise TerminalBrokerError("timeout_sec must be a positive integer")

        operation_id = str(uuid4())
        enqueued_at = time.monotonic()
        enqueued_wall = datetime.now(UTC)
        cancellation: asyncio.CancelledError | None = None
        async with self._execution_lock:
            started_at = time.monotonic()
            started_wall = datetime.now(UTC)
            try:
                result = await self._environment.exec(
                    command, cwd=cwd, timeout_sec=timeout_sec
                )
                error: str | None = None
            except asyncio.CancelledError as exc:
                result = None
                error = "CancelledError: terminal execution cancelled"
                cancellation = exc
            except Exception as exc:
                result = None
                error = f"{type(exc).__name__}: {exc}"
            ended_at = time.monotonic()
            ended_wall = datetime.now(UTC)

            record = {
                "schema_version": "1",
                "event": "terminal_exec",
                "operation_id": operation_id,
                "agent_id": agent_id,
                "command": command,
                "cwd": cwd,
                "timeout_sec": timeout_sec,
                "enqueued_at": enqueued_wall.isoformat(),
                "started_at": started_wall.isoformat(),
                "ended_at": ended_wall.isoformat(),
                "queue_wait_ms": (started_at - enqueued_at) * 1000,
                "execution_ms": (ended_at - started_at) * 1000,
                "stdout": result.stdout if result else None,
                "stderr": result.stderr if result else None,
                "return_code": result.return_code if result else None,
                "error": error,
            }
            await self._append_record(record)

        if cancellation is not None:
            raise cancellation
        return {"ok": error is None, **record}

    @staticmethod
    def _optional_str(request: dict[str, Any], field: str) -> str | None:
        value = request.get(field)
        if value is not None and not isinstance(value, str):
            raise TerminalBrokerError(f"{field} must be a string or null")
        return value

    async def _append_record(self, record: dict[str, Any]) -> None:
        line = json.dumps(record, ensure_ascii=False, separators=(",", ":")) + "\n"
        async with self._log_lock:
            with self.log_path.open("a", encoding="utf-8") as stream:
                stream.write(line)
                stream.flush()
