import io
import json

from harbor_buzz_orchestra import terminal_mcp


def test_mcp_protocol_exposes_buzz_to_all_and_exec_only_to_worker(
    monkeypatch, capsys, tmp_path
):
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
    terminal_mcp.serve(
        agent_id="worker-1",
        buzz_binary="/pinned/buzz",
        log_path=tmp_path / "orchestration.jsonl",
        socket_path=socket_path,
    )

    responses = [json.loads(line) for line in capsys.readouterr().out.splitlines()]
    assert responses[0]["result"]["protocolVersion"] == "2025-06-18"
    assert [tool["name"] for tool in responses[1]["result"]["tools"]] == [
        "buzz_exec",
        "exec",
    ]
    tool_result = json.loads(responses[2]["result"]["content"][0]["text"])
    assert tool_result["stdout"] == "hello"
    assert requests == [
        (socket_path, {"agent_id": "worker-1", "command": "echo hello"})
    ]

    monkeypatch.setattr(
        terminal_mcp.sys,
        "stdin",
        io.StringIO(json.dumps(messages[1]) + "\n"),
    )
    terminal_mcp.serve(
        agent_id="orch-1",
        buzz_binary="/pinned/buzz",
        log_path=tmp_path / "orchestration.jsonl",
    )
    orchestrator = json.loads(capsys.readouterr().out)
    assert [tool["name"] for tool in orchestrator["result"]["tools"]] == ["buzz_exec"]


def test_buzz_exec_prepends_pinned_binary_and_logs_span(monkeypatch, tmp_path):
    calls = []
    monkeypatch.setenv("BUZZ_RELAY_URL", "http://relay")
    monkeypatch.setenv("BUZZ_PRIVATE_KEY", "agent-secret")
    monkeypatch.setenv("BUZZ_AUTH_TAG", "agent-auth")
    monkeypatch.setenv("ANTHROPIC_API_KEY", "must-not-leak")

    def run(command, **kwargs):
        calls.append((command, kwargs))
        return terminal_mcp.subprocess.CompletedProcess(
            command, 0, stdout='{"event_id":"abc"}\n', stderr=""
        )

    monkeypatch.setattr(terminal_mcp.subprocess, "run", run)
    log_path = tmp_path / "orchestration.jsonl"
    result = terminal_mcp._buzz_exec(
        buzz_binary="/pinned/buzz",
        agent_id="worker-1",
        log_path=log_path,
        arguments={
            "args": [
                "messages",
                "send",
                "--channel",
                "trial-channel",
                "--content",
                "hi",
            ],
        },
    )

    assert result["ok"] is True
    assert calls[0][0] == [
        "/pinned/buzz",
        "messages",
        "send",
        "--channel",
        "trial-channel",
        "--content",
        "hi",
    ]
    assert calls[0][1]["timeout"] == 120
    assert set(calls[0][1]["env"]) <= {
        "BUZZ_RELAY_URL",
        "BUZZ_PRIVATE_KEY",
        "BUZZ_AUTH_TAG",
        "HOME",
        "PATH",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
    }
    assert calls[0][1]["env"]["BUZZ_PRIVATE_KEY"] == "agent-secret"
    assert "ANTHROPIC_API_KEY" not in calls[0][1]["env"]
    record = json.loads(log_path.read_text())
    assert record["event"] == "buzz_exec"
    assert record["agent_id"] == "worker-1"
    assert record["return_code"] == 0
