import asyncio
import json
import tempfile
from pathlib import Path

import pytest
from harbor.environments.base import ExecResult

from harbor_buzz_orchestra.terminal_broker import (
    SerializedTerminalBroker,
    TerminalBrokerError,
)


class ControlledEnvironment:
    def __init__(self):
        self.calls = []
        self.first_started = asyncio.Event()
        self.release_first = asyncio.Event()

    async def exec(self, command, cwd=None, timeout_sec=None):
        self.calls.append((command, cwd, timeout_sec))
        if len(self.calls) == 1:
            self.first_started.set()
            await self.release_first.wait()
        return ExecResult(stdout=f"out:{command}", stderr="", return_code=0)


@pytest.mark.asyncio
async def test_serializes_execution_and_logs_queue_wait(tmp_path: Path):
    environment = ControlledEnvironment()
    broker = SerializedTerminalBroker(
        environment=environment,
        socket_path=tmp_path / "broker.sock",
        log_path=tmp_path / "orchestration.jsonl",
    )

    first = asyncio.create_task(
        broker.execute({"agent_id": "worker-1", "command": "first"})
    )
    await environment.first_started.wait()
    second = asyncio.create_task(
        broker.execute({"agent_id": "worker-2", "command": "second"})
    )
    await asyncio.sleep(0.02)
    assert environment.calls == [("first", None, None)]

    environment.release_first.set()
    first_result, second_result = await asyncio.gather(first, second)

    assert environment.calls == [("first", None, None), ("second", None, None)]
    assert first_result["queue_wait_ms"] >= 0
    assert second_result["queue_wait_ms"] >= 15
    records = [
        json.loads(line)
        for line in (tmp_path / "orchestration.jsonl").read_text().splitlines()
    ]
    assert [record["command"] for record in records] == ["first", "second"]
    assert all("queue_wait_ms" in record for record in records)
    assert all("execution_ms" in record for record in records)


@pytest.mark.asyncio
async def test_socket_round_trip_and_cleanup(tmp_path: Path):
    environment = ControlledEnvironment()
    environment.release_first.set()
    socket_path = Path(tempfile.gettempdir()) / f"buzz-broker-{id(environment)}.sock"
    broker = SerializedTerminalBroker(
        environment=environment,
        socket_path=socket_path,
        log_path=tmp_path / "orchestration.jsonl",
    )

    async with broker:
        reader, writer = await asyncio.open_unix_connection(socket_path)
        writer.write(b'{"agent_id":"orchestrator-1","command":"pwd"}\n')
        await writer.drain()
        response = json.loads(await reader.readline())
        writer.close()
        await writer.wait_closed()
        assert response["ok"] is True
        assert response["stdout"] == "out:pwd"

    assert not socket_path.exists()


@pytest.mark.asyncio
async def test_logs_cancelled_execution_before_propagating(tmp_path: Path):
    environment = ControlledEnvironment()
    broker = SerializedTerminalBroker(
        environment=environment,
        socket_path=tmp_path / "broker.sock",
        log_path=tmp_path / "orchestration.jsonl",
    )
    task = asyncio.create_task(
        broker.execute({"agent_id": "worker-1", "command": "slow"})
    )
    await environment.first_started.wait()
    task.cancel()
    with pytest.raises(asyncio.CancelledError):
        await task
    record = json.loads((tmp_path / "orchestration.jsonl").read_text())
    assert record["error"] == "CancelledError: terminal execution cancelled"
    assert record["return_code"] is None


@pytest.mark.asyncio
async def test_rejects_invalid_request_without_execution(tmp_path: Path):
    environment = ControlledEnvironment()
    broker = SerializedTerminalBroker(
        environment=environment,
        socket_path=tmp_path / "broker.sock",
        log_path=tmp_path / "orchestration.jsonl",
    )
    with pytest.raises(TerminalBrokerError, match="command"):
        await broker.execute({"agent_id": "worker-1", "command": ""})
    assert environment.calls == []
