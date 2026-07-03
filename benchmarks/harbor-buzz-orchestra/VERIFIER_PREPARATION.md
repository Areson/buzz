# Offline verifier preparation

Terminal-Bench 2.1 verifiers commonly install Python dependencies at grading time. That is not a valid measurement path in Block's network environment, so benchmark tasks use generated **prepared images**.

The preparation contract is deliberately narrow:

- `tests/` remains byte-identical to the frozen source task.
- Python wheels are resolved host-side through Block Artifactory's `block-pypi`, then frozen in-repo by exact version and SHA-256.
- The source task image receives an additive dependency layer; the verifier still sees the task's full filesystem.
- Offline `apt-get`, installer `curl`, and `uvx` adapters only satisfy the source verifier's fixed bootstrap commands. Their PATH and `UV_OFFLINE=1` are verifier-phase environment variables, not agent defaults.
- `[verifier].network_mode = "no-network"` is mandatory and fail-closed.
- Metadata records the dependency lock, source-test bytes, base-image digest, and prepared-layer content hash. The scored runner must additionally record the built image digest.

Generate the hello-world M1 task:

```bash
uv run python -m harbor_buzz_orchestra.prepare_verifier \
  /path/to/harbor/examples/tasks/hello-world \
  /tmp/prepared-hello-world
```

A prepared image is visible to the agent and is therefore richer than the stock TB-2.1 image. Internal comparisons remain valid only when every benchmark arm uses the same prepared task. Absolute rewards have an external-comparability caveat and must be reported with `prepared_image: true` plus the layer content hash.

## Execution environment

Scored trials run on native Linux. Harbor 0.17 correctly rejects verifier phase `no-network` for Docker Desktop on macOS because that provider cannot promise enforceable phase network switching. The macOS development gate is an explicit `docker run --network none` verifier proof; the complete Harbor policy gate runs on the Linux benchmark runner.
