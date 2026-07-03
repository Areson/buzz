import io
import json

from harbor_buzz_orchestra import terminal_mcp


def test_mcp_protocol_exposes_exec_and_relays_result(monkeypatch, capsys, tmp_path):
    requests = []

    def broker_call(socket_path, request):
        requests.append((socket_path, request))
        return {"ok": True, "stdout": "hello", "stderr": "", "return_code": 0}

    monkeypatch.setattr(terminal_mcp, "_broker_call", broker_call)
    messages = [
        {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
        {
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {"name": "exec", "arguments": {"command": "echo hello"}},
        },
    ]
    monkeypatch.setattr(
        terminal_mcp.sys,
        "stdin",
        io.StringIO("".join(json.dumps(message) + "\n" for message in messages)),
    )

    socket_path = tmp_path / "broker.sock"
    terminal_mcp.serve(socket_path, "worker-1")

    responses = [json.loads(line) for line in capsys.readouterr().out.splitlines()]
    assert responses[0]["result"]["protocolVersion"] == "2025-06-18"
    assert responses[1]["result"]["tools"][0]["name"] == "exec"
    tool_result = json.loads(responses[2]["result"]["content"][0]["text"])
    assert tool_result["stdout"] == "hello"
    assert requests == [
        (socket_path, {"agent_id": "worker-1", "command": "echo hello"})
    ]
