"""Trial-scoped stdio MCP server for the real Buzz CLI and Harbor terminal."""

from __future__ import annotations

import argparse
import fcntl
import json
import os
import socket
import subprocess
import sys
import time
from datetime import UTC, datetime
from pathlib import Path
from typing import Any
from uuid import uuid4

BUZZ_EXEC_TOOL = {
    "name": "buzz_exec",
    "description": (
        "Run the production Buzz CLI as this trial agent. Pass arguments after "
        "the `buzz` executable, for example ['messages', 'send', '--channel', "
        "'<id>', '--content', 'hello']."
    ),
    "inputSchema": {
        "type": "object",
        "properties": {
            "args": {
                "type": "array",
                "items": {"type": "string"},
                "minItems": 1,
            },
            "stdin": {"type": ["string", "null"]},
            "timeout_sec": {
                "type": ["integer", "null"],
                "minimum": 1,
                "maximum": 120,
            },
        },
        "required": ["args"],
        "additionalProperties": False,
    },
}

EXEC_TOOL = {
    "name": "exec",
    "description": "Execute a shell command in the shared Harbor task environment.",
    "inputSchema": {
        "type": "object",
        "properties": {
            "command": {"type": "string", "minLength": 1},
            "cwd": {"type": ["string", "null"]},
            "timeout_sec": {"type": ["integer", "null"], "minimum": 1},
        },
        "required": ["command"],
        "additionalProperties": False,
    },
}


def _reply(message_id: Any, *, result: Any = None, error: Any = None) -> None:
    payload = {"jsonrpc": "2.0", "id": message_id}
    payload["error" if error is not None else "result"] = error or result
    print(json.dumps(payload, ensure_ascii=False), flush=True)


def _tool_result(response: dict[str, Any]) -> dict[str, Any]:
    return {
        "content": [{"type": "text", "text": json.dumps(response, ensure_ascii=False)}],
        "isError": not response.get("ok", False),
    }


def _broker_call(socket_path: Path, request: dict[str, Any]) -> dict[str, Any]:
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
        client.connect(str(socket_path))
        client.sendall(json.dumps(request, ensure_ascii=False).encode() + b"\n")
        with client.makefile("r", encoding="utf-8") as stream:
            raw = stream.readline()
    if not raw:
        raise RuntimeError("terminal broker closed without a response")
    response = json.loads(raw)
    if not isinstance(response, dict):
        raise RuntimeError("terminal broker returned a non-object response")
    return response


def _append_receipt(path: Path, receipt: dict[str, Any]) -> None:
    line = json.dumps(receipt, ensure_ascii=False, separators=(",", ":")) + "\n"
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8") as stream:
        fcntl.flock(stream, fcntl.LOCK_EX)
        stream.write(line)
        stream.flush()
        fcntl.flock(stream, fcntl.LOCK_UN)


def _buzz_exec(
    *,
    buzz_binary: str,
    agent_id: str,
    log_path: Path,
    arguments: dict[str, Any],
) -> dict[str, Any]:
    args = arguments.get("args")
    stdin = arguments.get("stdin")
    timeout_sec = arguments.get("timeout_sec")
    if (
        not isinstance(args, list)
        or not args
        or any(not isinstance(arg, str) for arg in args)
    ):
        raise ValueError("args must be a non-empty array of strings")
    if stdin is not None and not isinstance(stdin, str):
        raise ValueError("stdin must be a string or null")
    if timeout_sec is not None and (
        isinstance(timeout_sec, bool)
        or not isinstance(timeout_sec, int)
        or not 1 <= timeout_sec <= 120
    ):
        raise ValueError("timeout_sec must be an integer from 1 through 120")

    operation_id = str(uuid4())
    started_wall = datetime.now(UTC)
    started_at = time.monotonic()
    try:
        completed = subprocess.run(
            [buzz_binary, *args],
            input=stdin,
            text=True,
            capture_output=True,
            timeout=timeout_sec or 120,
            check=False,
            env={
                key: value
                for key, value in os.environ.items()
                if key
                in {
                    "BUZZ_RELAY_URL",
                    "BUZZ_PRIVATE_KEY",
                    "BUZZ_AUTH_TAG",
                    "HOME",
                    "PATH",
                    "SSL_CERT_FILE",
                    "SSL_CERT_DIR",
                }
            },
        )
        stdout = completed.stdout
        stderr = completed.stderr
        return_code = completed.returncode
        error = None if return_code == 0 else f"buzz exited {return_code}"
    except subprocess.TimeoutExpired as exc:
        stdout = exc.stdout or ""
        stderr = exc.stderr or ""
        return_code = None
        error = f"buzz timed out after {timeout_sec or 120} seconds"
    ended_at = time.monotonic()
    ended_wall = datetime.now(UTC)
    record = {
        "schema_version": "1",
        "event": "buzz_exec",
        "operation_id": operation_id,
        "agent_id": agent_id,
        "args": args,
        "started_at": started_wall.isoformat(),
        "ended_at": ended_wall.isoformat(),
        "execution_ms": (ended_at - started_at) * 1000,
        "stdout": stdout,
        "stderr": stderr,
        "return_code": return_code,
        "error": error,
    }
    _append_receipt(log_path, record)
    return {"ok": error is None, **record}


def serve(
    *,
    agent_id: str,
    buzz_binary: str,
    log_path: Path,
    socket_path: Path | None = None,
) -> None:
    tools = [BUZZ_EXEC_TOOL]
    if socket_path is not None:
        tools.append(EXEC_TOOL)
    for raw in sys.stdin:
        message: Any = None
        try:
            message = json.loads(raw)
            method = message.get("method")
            message_id = message.get("id")
            if message_id is None:  # notification
                continue
            if method == "initialize":
                _reply(
                    message_id,
                    result={
                        "protocolVersion": "2025-06-18",
                        "capabilities": {"tools": {}},
                        "serverInfo": {
                            "name": "harbor-buzz-agent-tools",
                            "version": "0.1.0",
                        },
                    },
                )
            elif method == "tools/list":
                _reply(message_id, result={"tools": tools})
            elif method == "tools/call":
                params = message.get("params") or {}
                arguments = params.get("arguments") or {}
                if params.get("name") == "buzz_exec":
                    response = _buzz_exec(
                        buzz_binary=buzz_binary,
                        agent_id=agent_id,
                        log_path=log_path,
                        arguments=arguments,
                    )
                elif params.get("name") == "exec" and socket_path is not None:
                    response = _broker_call(
                        socket_path, {"agent_id": agent_id, **arguments}
                    )
                else:
                    raise ValueError("unknown tool")
                _reply(message_id, result=_tool_result(response))
            else:
                _reply(
                    message_id,
                    error={"code": -32601, "message": f"unknown method: {method}"},
                )
        except Exception as error:
            _reply(
                message.get("id") if isinstance(message, dict) else None,
                error={"code": -32000, "message": str(error)},
            )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--agent-id", required=True)
    parser.add_argument("--buzz-binary", required=True)
    parser.add_argument("--log-path", type=Path, required=True)
    parser.add_argument("--socket", type=Path)
    args = parser.parse_args()
    serve(
        agent_id=args.agent_id,
        buzz_binary=args.buzz_binary,
        log_path=args.log_path,
        socket_path=args.socket,
    )


if __name__ == "__main__":
    main()
