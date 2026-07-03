"""Production-path Buzz ACP subprocess runtime for Harbor trials."""

from __future__ import annotations

import asyncio
import json
import os
import shlex
import signal
import sys
import tempfile
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from harbor.environments.base import BaseEnvironment

from .manifest import AgentClass, ExperimentManifest
from .provisioning import AgentCredential, TrialHandle
from .runtime import RuntimeResult
from .terminal_broker import SerializedTerminalBroker


DEFAULT_MAX_AGENT_ROUNDS = 32


class RuntimeLaunchError(RuntimeError):
    """Raised when a Buzz agent process cannot be launched or exits early."""


@dataclass(frozen=True, slots=True)
class EndpointLaunchConfig:
    """Deployment-specific environment needed to launch one manifest endpoint."""

    provider: str
    api_key_env: str
    env: dict[str, str] = field(default_factory=dict)


@dataclass(slots=True)
class _AgentProcess:
    credential: AgentCredential
    process: asyncio.subprocess.Process
    stdout_path: Path
    stderr_path: Path
    stdout_stream: Any
    stderr_stream: Any


class BuzzSubprocessRuntime:
    """Run one ``buzz-acp``/``buzz-agent`` pair per provisioned identity."""

    def __init__(
        self,
        *,
        logs_dir: Path,
        artifact_root: Path,
        endpoints: dict[str, EndpointLaunchConfig],
        buzz_acp_binary: str = "buzz-acp",
        buzz_agent_binary: str = "buzz-agent",
        buzz_cli_binary: str = "buzz",
        max_agent_rounds: int = DEFAULT_MAX_AGENT_ROUNDS,
        readiness_timeout_seconds: float = 30.0,
        poll_seconds: float = 1.0,
    ) -> None:
        if max_agent_rounds <= 0:
            raise ValueError("max_agent_rounds must be positive")
        if readiness_timeout_seconds <= 0:
            raise ValueError("readiness_timeout_seconds must be positive")
        self.logs_dir = Path(logs_dir)
        self.artifact_root = Path(artifact_root)
        self.endpoints = endpoints
        self.buzz_acp_binary = buzz_acp_binary
        self.buzz_agent_binary = buzz_agent_binary
        self.buzz_cli_binary = buzz_cli_binary
        self.max_agent_rounds = max_agent_rounds
        self.readiness_timeout_seconds = readiness_timeout_seconds
        self.poll_seconds = poll_seconds

    async def run(
        self,
        *,
        instruction: str,
        environment: BaseEnvironment,
        manifest: ExperimentManifest,
        trial: TrialHandle,
    ) -> RuntimeResult:
        classes = self._classes_by_agent_id(manifest, trial.credentials)
        orchestrator = next(c for c in trial.credentials if c.role == "orchestrator")
        workers = [c for c in trial.credentials if c.agent_id != orchestrator.agent_id]
        if not workers:
            raise RuntimeLaunchError("Buzz orchestration requires at least one worker")
        trial_dir = self.logs_dir / "buzz"
        trial_dir.mkdir(parents=True, exist_ok=True)
        socket_path = Path(tempfile.gettempdir()) / f"hb-{trial.trial_id[:12]}.sock"
        processes: list[_AgentProcess] = []

        broker = SerializedTerminalBroker(
            environment=environment,
            socket_path=socket_path,
            log_path=self.logs_dir / "orchestration.jsonl",
        )
        async with broker:
            try:
                await self._buzz_json(
                    trial.user,
                    trial,
                    "users",
                    "set-profile",
                    "--name",
                    trial.user.agent_id,
                )
                for credential in trial.credentials:
                    await self._buzz_json(
                        credential,
                        trial,
                        "users",
                        "set-profile",
                        "--name",
                        credential.agent_id,
                    )
                    processes.append(
                        await self._launch_agent(
                            trial=trial,
                            credential=credential,
                            agent_class=classes[credential.agent_id],
                            socket_path=socket_path,
                            trial_dir=trial_dir,
                        )
                    )
                await self._wait_for_agents_ready(processes, trial.channel_id)
                # The task arrives exactly as it would in production Buzz: a
                # user prompt @mentioning the orchestrator. The harness never
                # speaks as any agent.
                await self._send(
                    trial.user,
                    trial,
                    f"@{orchestrator.agent_id} {instruction}",
                )
                final_message = await asyncio.wait_for(
                    self._wait_for_done(orchestrator, trial, processes),
                    timeout=manifest.trial_budget.timeout_seconds,
                )
                await self._verify_m1_output(environment, manifest)
            finally:
                await self._stop_processes(processes)

        return RuntimeResult(
            metadata={
                "completion_message_id": final_message["id"],
                "completion_message": final_message["content"],
                "terminal_concurrency": "serialized",
                "agent_hints_enabled": False,
                "task_seed": "user-identity-prompt",
                "agent_max_rounds": {
                    credential.agent_id: (
                        classes[credential.agent_id].budget.max_calls
                        or self.max_agent_rounds
                    )
                    for credential in trial.credentials
                },
            }
        )

    async def _launch_agent(
        self,
        *,
        trial: TrialHandle,
        credential: AgentCredential,
        agent_class: AgentClass,
        socket_path: Path,
        trial_dir: Path,
    ) -> _AgentProcess:
        if not credential.llm_endpoint:
            raise RuntimeLaunchError("credential llm_endpoint must not be empty")
        endpoint = self.endpoints.get(credential.llm_endpoint)
        if endpoint is None:
            raise RuntimeLaunchError(
                f"no launch config for endpoint {credential.llm_endpoint!r}"
            )
        prompt_path = self.artifact_root / agent_class.prompt.path
        self._verify_artifact(prompt_path, agent_class.prompt.sha256)
        composed_prompt_path = self._compose_system_prompt(
            trial_dir=trial_dir,
            trial=trial,
            credential=credential,
            persona_path=prompt_path,
        )
        wrapper = self._write_mcp_wrapper(
            trial_dir=trial_dir,
            agent_id=credential.agent_id,
            socket_path=socket_path if credential.role == "worker" else None,
        )
        stdout_path = trial_dir / f"{credential.agent_id}.stdout.log"
        stderr_path = trial_dir / f"{credential.agent_id}.stderr.log"
        stdout_stream = stdout_path.open("wb")
        stderr_stream = stderr_path.open("wb")
        env = {
            **os.environ,
            **endpoint.env,
            "BUZZ_RELAY_URL": trial.relay_ws_url,
            "BUZZ_PRIVATE_KEY": credential.nostr_secret_key,
            "BUZZ_AUTH_TAG": credential.nostr_auth_tag,
            "BUZZ_ACP_AGENT_COMMAND": self.buzz_agent_binary,
            "BUZZ_ACP_AGENT_ARGS": "",
            "BUZZ_ACP_CHANNELS": trial.channel_id,
            "BUZZ_ACP_SUBSCRIBE": "mentions",
            "BUZZ_ACP_RESPOND_TO": "anyone",
            "BUZZ_ACP_NO_MEMORY": "true",
            "BUZZ_ACP_SYSTEM_PROMPT_FILE": str(composed_prompt_path),
            "BUZZ_AGENT_PROVIDER": endpoint.provider,
            "BUZZ_AGENT_MODEL": credential.llm_endpoint,
            "BUZZ_AGENT_MAX_OUTPUT_TOKENS": str(
                agent_class.generation.max_output_tokens
            ),
            "BUZZ_AGENT_MAX_CONTEXT_TOKENS": str(
                agent_class.generation.context_window_tokens
            ),
            # Benchmark context is manifest-owned. Disable host cwd/home hint and
            # skill discovery so trials cannot inherit operator-specific tools.
            "BUZZ_AGENT_NO_HINTS": "1",
            "BUZZ_AGENT_MAX_ROUNDS": str(
                agent_class.budget.max_calls or self.max_agent_rounds
            ),
            endpoint.api_key_env: credential.llm_api_key,
        }
        env["BUZZ_ACP_MCP_COMMAND"] = str(wrapper)
        self._reject_identity_overrides(endpoint)
        try:
            process = await asyncio.create_subprocess_exec(
                self.buzz_acp_binary,
                stdout=stdout_stream,
                stderr=stderr_stream,
                env=env,
                start_new_session=True,
            )
        except Exception:
            stdout_stream.close()
            stderr_stream.close()
            raise
        return _AgentProcess(
            credential, process, stdout_path, stderr_path, stdout_stream, stderr_stream
        )

    async def _wait_for_agents_ready(
        self, processes: list[_AgentProcess], channel_id: str
    ) -> None:
        """Wait until every ACP process confirms its trial-channel subscription."""
        marker = f"subscribed to channel {channel_id}"
        deadline = asyncio.get_running_loop().time() + self.readiness_timeout_seconds
        pending = {item.credential.agent_id: item for item in processes}
        while pending:
            self._raise_for_early_exit(processes)
            for agent_id, item in list(pending.items()):
                try:
                    output = item.stdout_path.read_text(
                        encoding="utf-8", errors="replace"
                    )
                except OSError as error:
                    raise RuntimeLaunchError(
                        f"cannot read readiness log for agent {agent_id}: {error}"
                    ) from error
                if marker in output:
                    del pending[agent_id]
            if not pending:
                return
            if asyncio.get_running_loop().time() >= deadline:
                raise RuntimeLaunchError(
                    "agents did not subscribe to trial channel before readiness "
                    f"timeout: {sorted(pending)}"
                )
            await asyncio.sleep(self.poll_seconds)

    @staticmethod
    async def _verify_m1_output(
        environment: BaseEnvironment, manifest: ExperimentManifest
    ) -> None:
        """Fail M1 immediately unless the artifact satisfies the grader contract."""
        if manifest.condition != "M1-hello-world":
            return
        result = await environment.exec(
            'python3 -c "from pathlib import Path; '
            "p = Path('/app/hello.txt'); "
            "assert p.is_file() and p.read_text().strip() == 'Hello, world!'\""
        )
        if result.return_code != 0:
            detail = (
                result.stderr or result.stdout or "grader-equivalent check failed"
            ).strip()
            raise RuntimeLaunchError(
                "M1 pre-verifier sanity probe failed: /app/hello.txt must exist "
                f"and its stripped text must equal 'Hello, world!' ({detail})"
            )

    async def _wait_for_done(
        self,
        orchestrator: AgentCredential,
        trial: TrialHandle,
        processes: list[_AgentProcess],
    ) -> dict[str, Any]:
        """Observe the channel as the trial user until the orchestrator posts DONE.

        Observation only: the harness never speaks as any agent. If the team
        stalls, the trial times out and the stall is the measured result.
        """
        while True:
            self._raise_for_early_exit(processes)
            messages = await self._buzz_json(
                trial.user,
                trial,
                "messages",
                "get",
                "--channel",
                trial.channel_id,
                "--limit",
                "100",
            )
            for message in messages:
                if message.get("pubkey") == orchestrator.nostr_pubkey and str(
                    message.get("content", "")
                ).startswith("DONE:"):
                    return message
            await asyncio.sleep(self.poll_seconds)

    async def _send(
        self, credential: AgentCredential, trial: TrialHandle, content: str
    ) -> None:
        await self._buzz_json(
            credential,
            trial,
            "messages",
            "send",
            "--channel",
            trial.channel_id,
            "--content",
            content,
        )

    async def _buzz_json(
        self, credential: AgentCredential, trial: TrialHandle, *args: str
    ) -> Any:
        relay_url = self._cli_relay_url(trial.relay_ws_url)
        process = await asyncio.create_subprocess_exec(
            self.buzz_cli_binary,
            *args,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            env={
                **os.environ,
                "BUZZ_RELAY_URL": relay_url,
                "BUZZ_PRIVATE_KEY": credential.nostr_secret_key,
                "BUZZ_AUTH_TAG": credential.nostr_auth_tag,
            },
        )
        stdout, stderr = await process.communicate()
        if process.returncode != 0:
            raise RuntimeLaunchError(
                f"buzz {shlex.join(args)} exited {process.returncode}: "
                f"{stderr.decode(errors='replace').strip()}"
            )
        try:
            return json.loads(stdout)
        except json.JSONDecodeError as error:
            raise RuntimeLaunchError("buzz returned invalid JSON") from error

    @staticmethod
    def _cli_relay_url(relay_ws_url: str) -> str:
        if relay_ws_url.startswith("ws://"):
            return f"http://{relay_ws_url.removeprefix('ws://')}"
        if relay_ws_url.startswith("wss://"):
            return f"https://{relay_ws_url.removeprefix('wss://')}"
        raise RuntimeLaunchError("trial relay_ws_url must use ws:// or wss://")

    @staticmethod
    def _classes_by_agent_id(
        manifest: ExperimentManifest, credentials: tuple[AgentCredential, ...]
    ) -> dict[str, AgentClass]:
        by_id = {entry.id: entry for entry in manifest.roster}
        result: dict[str, AgentClass] = {}
        for credential in credentials:
            class_id, separator, index = credential.agent_id.rpartition("-")
            match = by_id.get(class_id)
            if not separator or not index.isdigit() or match is None:
                raise RuntimeLaunchError(
                    f"credential {credential.agent_id!r} does not match a roster class"
                )
            if credential.role != match.kind:
                raise RuntimeLaunchError(
                    f"credential {credential.agent_id!r} role does not match manifest"
                )
            result[credential.agent_id] = match
        return result

    @staticmethod
    def _verify_artifact(path: Path, expected_sha256: str) -> None:
        import hashlib

        try:
            actual = hashlib.sha256(path.read_bytes()).hexdigest()
        except OSError as error:
            raise RuntimeLaunchError(f"cannot read prompt {path}: {error}") from error
        if actual != expected_sha256:
            raise RuntimeLaunchError(
                f"prompt hash mismatch for {path}: expected {expected_sha256}, got {actual}"
            )

    def _compose_system_prompt(
        self,
        *,
        trial_dir: Path,
        trial: TrialHandle,
        credential: AgentCredential,
        persona_path: Path,
    ) -> Path:
        """Append the trial's team roster to the pinned persona.

        The analogue of a production Buzz workspace's team context: each agent
        knows its own identity, its channel, the user it reports to, and its
        teammates' names, pubkeys, and roles from its system prompt — it never
        has to discover them over the relay.
        """
        persona = persona_path.read_text(encoding="utf-8")
        lines = [
            "",
            "## Your team",
            "",
            f"You are `{credential.agent_id}` (pubkey `{credential.nostr_pubkey}`).",
            f"The team coordinates in Buzz channel `{trial.channel_id}`.",
            f"Tasks come from the user `{trial.user.agent_id}` "
            f"(pubkey `{trial.user.nostr_pubkey}`); address your final report "
            "to them.",
            "",
            "| Name | Role | Pubkey |",
            "|------|------|--------|",
        ]
        for teammate in trial.credentials:
            if teammate.agent_id == credential.agent_id:
                continue
            lines.append(
                f"| {teammate.agent_id} | {teammate.role} "
                f"| `{teammate.nostr_pubkey}` |"
            )
        composed = persona + "\n".join(lines) + "\n"
        path = trial_dir / f"{credential.agent_id}.system-prompt.md"
        path.write_text(composed, encoding="utf-8")
        path.chmod(0o600)
        return path

    def _write_mcp_wrapper(
        self, *, trial_dir: Path, agent_id: str, socket_path: Path | None
    ) -> Path:
        path = trial_dir / f"agent-mcp-{agent_id}"
        log_path = self.logs_dir / "orchestration.jsonl"
        socket_argument = (
            f", socket_path=Path({str(socket_path)!r})"
            if socket_path is not None
            else ""
        )
        command = (
            f"#!{sys.executable}\n"
            "from harbor_buzz_orchestra.terminal_mcp import serve\n"
            "from pathlib import Path\n"
            "serve("
            f"agent_id={agent_id!r}, "
            f"buzz_binary={self.buzz_cli_binary!r}, "
            f"log_path=Path({str(log_path)!r})"
            f"{socket_argument})\n"
        )
        path.write_text(command, encoding="utf-8")
        path.chmod(0o700)
        return path

    @staticmethod
    def _reject_identity_overrides(endpoint: EndpointLaunchConfig) -> None:
        forbidden = {
            "BUZZ_RELAY_URL",
            "BUZZ_PRIVATE_KEY",
            "BUZZ_AUTH_TAG",
            "BUZZ_ACP_CHANNELS",
        }
        overlap = forbidden & endpoint.env.keys()
        if overlap:
            raise RuntimeLaunchError(
                f"endpoint env cannot override trial identity: {sorted(overlap)}"
            )

    @staticmethod
    def _raise_for_early_exit(processes: list[_AgentProcess]) -> None:
        for item in processes:
            if item.process.returncode is not None:
                raise RuntimeLaunchError(
                    f"agent {item.credential.agent_id} exited "
                    f"{item.process.returncode}; see {item.stderr_path}"
                )

    @staticmethod
    async def _stop_processes(processes: list[_AgentProcess]) -> None:
        for item in processes:
            if item.process.returncode is None:
                try:
                    os.killpg(item.process.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
        for item in processes:
            if item.process.returncode is None:
                try:
                    await asyncio.wait_for(item.process.wait(), timeout=5)
                except TimeoutError:
                    try:
                        os.killpg(item.process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    await item.process.wait()
            item.stdout_stream.close()
            item.stderr_stream.close()
