import hashlib
import json
from pathlib import Path

import pytest

from harbor_buzz_orchestra.manifest import ExperimentManifest
from harbor_buzz_orchestra.provisioning import AgentCredential, TrialHandle
from harbor_buzz_orchestra.subprocess_runtime import (
    BuzzSubprocessRuntime,
    EndpointLaunchConfig,
    RuntimeLaunchError,
)


def write_manifest(tmp_path: Path) -> ExperimentManifest:
    prompt = tmp_path / "prompt.md"
    prompt.write_text("prompt", encoding="utf-8")
    digest = hashlib.sha256(prompt.read_bytes()).hexdigest()
    return ExperimentManifest.load(
        {
            "condition": "test",
            "roster": [
                {
                    "id": "orch",
                    "kind": "orchestrator",
                    "role": "lead",
                    "count": 1,
                    "endpoint": "orch-model",
                    "model_revision": "r1",
                    "prompt": {"path": "prompt.md", "sha256": digest},
                    "generation": {
                        "max_output_tokens": 100,
                        "context_window_tokens": 1000,
                    },
                },
                {
                    "id": "worker",
                    "kind": "worker",
                    "role": "implementer",
                    "count": 1,
                    "endpoint": "worker-model",
                    "model_revision": "r1",
                    "prompt": {"path": "prompt.md", "sha256": digest},
                    "generation": {
                        "max_output_tokens": 100,
                        "context_window_tokens": 1000,
                    },
                },
            ],
            "prices": {
                "orch-model": {
                    "input_per_million_usd": 0,
                    "cached_input_per_million_usd": 0,
                    "output_per_million_usd": 0,
                },
                "worker-model": {
                    "input_per_million_usd": 0,
                    "cached_input_per_million_usd": 0,
                    "output_per_million_usd": 0,
                },
            },
            "trial_budget": {"timeout_seconds": 30},
        }
    )


def credential(agent_id, role, endpoint):
    return AgentCredential(
        agent_id=agent_id,
        role=role,
        nostr_secret_key=f"secret-{agent_id}",
        nostr_pubkey=f"pubkey-{agent_id}",
        nostr_auth_tag="[]",
        llm_endpoint=endpoint,
        llm_api_key=f"key-{agent_id}",
    )


def runtime(tmp_path, **kwargs):
    return BuzzSubprocessRuntime(
        logs_dir=tmp_path / "logs",
        artifact_root=tmp_path,
        endpoints={
            "orch-model": EndpointLaunchConfig("anthropic", "ANTHROPIC_API_KEY"),
            "worker-model": EndpointLaunchConfig("anthropic", "ANTHROPIC_API_KEY"),
        },
        **kwargs,
    )


def test_maps_credentials_exactly_and_rejects_role_mismatch(tmp_path):
    manifest = write_manifest(tmp_path)
    credentials = (
        credential("orch-1", "orchestrator", "orch-model"),
        credential("worker-1", "worker", "worker-model"),
    )
    assert set(runtime(tmp_path)._classes_by_agent_id(manifest, credentials)) == {
        "orch-1",
        "worker-1",
    }
    bad = (credential("worker-1", "orchestrator", "worker-model"),)
    with pytest.raises(RuntimeLaunchError, match="role"):
        runtime(tmp_path)._classes_by_agent_id(manifest, bad)


def test_prompt_hash_and_identity_override_are_fail_closed(tmp_path):
    manifest = write_manifest(tmp_path)
    prompt_ref = manifest.roster[0].prompt
    runtime(tmp_path)._verify_artifact(tmp_path / prompt_ref.path, prompt_ref.sha256)
    (tmp_path / prompt_ref.path).write_text("changed", encoding="utf-8")
    with pytest.raises(RuntimeLaunchError, match="hash mismatch"):
        runtime(tmp_path)._verify_artifact(
            tmp_path / prompt_ref.path, prompt_ref.sha256
        )

    endpoint = EndpointLaunchConfig(
        "anthropic", "ANTHROPIC_API_KEY", {"BUZZ_PRIVATE_KEY": "bad"}
    )
    with pytest.raises(RuntimeLaunchError, match="identity"):
        runtime(tmp_path)._reject_identity_overrides(endpoint)


def test_relay_url_conversion_is_explicit(tmp_path):
    rt = runtime(tmp_path)
    assert rt._cli_relay_url("ws://relay:3000") == "http://relay:3000"
    assert rt._cli_relay_url("wss://relay") == "https://relay"
    with pytest.raises(RuntimeLaunchError, match="ws://"):
        rt._cli_relay_url("http://relay")


def test_mcp_wrapper_pins_agent_and_socket(tmp_path):
    wrapper = runtime(tmp_path)._write_mcp_wrapper(
        tmp_path, "worker-1", tmp_path / "broker.sock"
    )
    content = wrapper.read_text()
    assert "worker-1" in content
    assert str(tmp_path / "broker.sock") in content
    assert wrapper.stat().st_mode & 0o777 == 0o700


@pytest.mark.asyncio
@pytest.mark.parametrize(("configured", "expected"), [(None, "32"), (7, "7")])
async def test_launch_sets_bounded_agent_rounds(
    tmp_path, monkeypatch, configured, expected
):
    manifest = write_manifest(tmp_path)
    agent_class = manifest.roster[0]
    if configured is not None:
        agent_class = agent_class.model_copy(
            update={
                "budget": agent_class.budget.model_copy(
                    update={"max_calls": configured}
                )
            }
        )
    orch = credential("orch-1", "orchestrator", "orch-model")
    trial = TrialHandle(
        run_id="run",
        trial_id="trial",
        manifest_hash="hash",
        relay_ws_url="ws://relay",
        channel_id="channel",
        credentials=(orch,),
    )
    captured = {}

    class Process:
        returncode = None

    async def create_subprocess_exec(*args, **kwargs):
        captured.update(kwargs["env"])
        return Process()

    monkeypatch.setattr(
        "harbor_buzz_orchestra.subprocess_runtime.asyncio.create_subprocess_exec",
        create_subprocess_exec,
    )
    launched = await runtime(tmp_path)._launch_agent(
        trial=trial,
        credential=orch,
        agent_class=agent_class,
        socket_path=tmp_path / "broker.sock",
        trial_dir=tmp_path,
    )
    launched.stdout_stream.close()
    launched.stderr_stream.close()

    assert captured["BUZZ_AGENT_NO_HINTS"] == "1"
    assert captured["BUZZ_AGENT_MAX_ROUNDS"] == expected


def test_runtime_rejects_unbounded_agent_rounds(tmp_path):
    with pytest.raises(ValueError, match="positive"):
        runtime(tmp_path, max_agent_rounds=0)


@pytest.mark.asyncio
async def test_wait_for_done_requires_orchestrator_authorship(tmp_path, monkeypatch):
    rt = runtime(tmp_path)
    orch = credential("orch-1", "orchestrator", "orch-model")
    trial = TrialHandle(
        run_id="run",
        trial_id="trial",
        manifest_hash="hash",
        relay_ws_url="ws://relay",
        channel_id="channel",
        credentials=(orch,),
    )
    rounds = iter(
        [
            [{"id": "1", "pubkey": "someone-else", "content": "DONE: fake"}],
            [{"id": "2", "pubkey": orch.nostr_pubkey, "content": "DONE: real"}],
        ]
    )

    async def buzz_json(*args, **kwargs):
        return next(rounds)

    async def no_sleep(_):
        return None

    monkeypatch.setattr(rt, "_buzz_json", buzz_json)
    monkeypatch.setattr(
        "harbor_buzz_orchestra.subprocess_runtime.asyncio.sleep", no_sleep
    )
    result = await rt._wait_for_done(orch, trial, [])
    assert json.dumps(result).find("real") > 0
