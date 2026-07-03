# Harbor Buzz Orchestra

A stock-Harbor custom agent that runs a manifest-defined team through the real
Buzz stack. Harbor sees one `BuzzOrchestraAgent`; behind that adapter, one
orchestrator and N workers coordinate over the production relay/Postgres using
pinned `buzz`, `buzz-acp`, and `buzz-agent` binaries. Workers share serialized
access to the Harbor task terminal. No Harbor fork or patch is required.

## Define the team

The manifest is the benchmark condition. Each roster entry selects an agent
class's count, model endpoint, byte-pinned system prompt, generation settings,
and budget:

```yaml
condition: my-team
roster:
  - id: orch
    kind: orchestrator
    role: lead
    count: 1
    endpoint: databricks/frontier
    prompt: {path: personas/orchestrator.md, sha256: <sha256>}
    generation: {max_output_tokens: 4096, context_window_tokens: 128000}
  - id: worker
    kind: worker
    role: implementer
    count: 4
    endpoint: databricks/fast-worker
    prompt: {path: personas/worker.md, sha256: <sha256>}
    generation: {max_output_tokens: 4096, context_window_tokens: 128000}
```

`endpoint_config` maps those endpoint names to providers, URLs, and API-key
environment variables. The adapter contains no fixed roster or model.

## Run

With the production compose stack and model endpoints already running, execute
one task (`-p`), a directory of tasks, or replace `-p` with Harbor's dataset and
task selectors:

```bash
uv run --project benchmarks/harbor-buzz-orchestra/testbed harbor run --yes -p <TASK_OR_DIRECTORY> --agent harbor_buzz_orchestra:BuzzOrchestraAgent --agent-kwarg manifest=<CONDITION.yaml> --agent-kwarg provisioner_factory=harbor_buzz_testbed:provisioner_from_dict --agent-kwarg provisioner_config=<PROVISIONER.json> --agent-kwarg endpoint_config=<ENDPOINTS.json> --agent-kwarg artifact_root=benchmarks/harbor-buzz-orchestra --agent-kwarg buzz_acp_binary=target/debug/buzz-acp --agent-kwarg buzz_agent_binary=target/debug/buzz-agent --agent-kwarg buzz_cli_binary=target/debug/buzz --agent-kwarg run_id="bench-$(date -u +%Y%m%dT%H%M%SZ)" --agent-timeout-multiplier 15 --n-concurrent 1
```

`--n-concurrent 1` is the safe laptop setting for a serialized local model; it
is not an orchestration requirement. Tasks whose graders install dependencies
at verification time must first be prepared for offline execution; see
[VERIFIER_PREPARATION.md](VERIFIER_PREPARATION.md).

Each trial gets fresh keys and a private Buzz channel. The provisioner archives
rather than deletes that channel, leaving the relay/Postgres event timeline and
`orchestration.jsonl` receipts available for analysis.

## Validate

```bash
cd benchmarks/harbor-buzz-orchestra
uv run --frozen --extra dev pytest -q
uv run --frozen --extra dev ruff check .
cd testbed
uv run --frozen --extra dev pytest -q
uv run --frozen --extra dev ruff check .
```

Live provisioner tests require the benchmark compose stack and opt-in
environment described in `testbed/tests/test_provisioner_live.py`.
