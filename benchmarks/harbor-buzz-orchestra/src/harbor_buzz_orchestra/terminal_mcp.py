"""Minimal stdio MCP server exposing the Harbor terminal broker."""

from __future__ import annotations

import argparse
import json
import socket
import sys
from pathlib import Path
from typing import Any

TOOL = {
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


def serve(socket_path: Path, agent_id: str) -> None:
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
                            "name": "harbor-terminal",
                            "version": "0.1.0",
                        },
                    },
                )
            elif method == "tools/list":
                _reply(message_id, result={"tools": [TOOL]})
            elif method == "tools/call":
                params = message.get("params") or {}
                if params.get("name") != "exec":
                    raise ValueError("unknown tool")
                arguments = params.get("arguments") or {}
                response = _broker_call(
                    socket_path, {"agent_id": agent_id, **arguments}
                )
                _reply(
                    message_id,
                    result={
                        "content": [
                            {
                                "type": "text",
                                "text": json.dumps(response, ensure_ascii=False),
                            }
                        ],
                        "isError": not response.get("ok", False),
                    },
                )
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
    parser.add_argument("--socket", type=Path, required=True)
    parser.add_argument("--agent-id", required=True)
    args = parser.parse_args()
    serve(args.socket, args.agent_id)


if __name__ == "__main__":
    main()
