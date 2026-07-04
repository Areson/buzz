#!/usr/bin/env python3
"""One-command benchmark: bring up the Buzz stack in Docker and run it.

``just benchmark`` wraps this script. Defaults are leaderboard-eligible out
of the box (Terminal-Bench 2.1, 5 attempts per problem, the Sonnet+Haiku
team); every ``run_leaderboard.py`` selector passes through unchanged. The
script owns everything around the run:

- A dedicated ``buzz-benchmark`` compose project reusing the production
  bundle (``deploy/compose/compose.yml``) plus the benchmark port overlay,
  on its own ports (relay :3600, Postgres :5633, metrics :9602) so it never
  collides with a dev stack. Secrets and identities are generated once into
  the gitignored ``.benchmark/`` state dir and reused across runs.
- One pinned *user* identity for the whole benchmark environment: it owns
  every trial channel and posts every task, like one human running many
  teams. Channels are kept (not archived) after each trial.
- ``--gui`` adds that user to the relay membership list and opens the Buzz
  desktop app logged in as them, so a human can watch the teams work live.

Run inside the testbed environment (the just recipe does this):

    uv run --project benchmarks/harbor-buzz-orchestra/testbed \
        benchmarks/harbor-buzz-orchestra/scripts/benchmark.py [--gui] [...]
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import secrets
import subprocess
import sys
import time
from pathlib import Path

PACKAGE_ROOT = Path(__file__).resolve().parent.parent
REPO_ROOT = PACKAGE_ROOT.parents[1]
STATE_DIR = PACKAGE_ROOT / ".benchmark"

COMPOSE_PROJECT = "buzz-benchmark"
COMPOSE_FILES = (
    REPO_ROOT / "deploy" / "compose" / "compose.yml",
    PACKAGE_ROOT / "testbed" / "compose.benchmark.yml",
)
RELAY_HTTP_PORT = 3600
PG_HOST_PORT = 5633
METRICS_HOST_PORT = 9602

DEFAULT_DATASET = "terminal-bench/terminal-bench-2-1"
DEFAULT_ATTEMPTS = 5
DEFAULT_MANIFEST = PACKAGE_ROOT / "manifests" / "tb-cobol-sonnet-haiku.yaml"
DEFAULT_ENDPOINTS = PACKAGE_ROOT / "testbed" / "endpoints" / "anthropic-live.json"
SCHEMA_SQL = PACKAGE_ROOT / "testbed" / "sql" / "benchmark_schema.sql"

_spec = importlib.util.spec_from_file_location(
    "run_leaderboard", Path(__file__).resolve().parent / "run_leaderboard.py"
)
run_leaderboard = importlib.util.module_from_spec(_spec)
sys.modules.setdefault("run_leaderboard", run_leaderboard)
_spec.loader.exec_module(run_leaderboard)


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=__doc__.splitlines()[0],
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    problems = parser.add_mutually_exclusive_group()
    problems.add_argument(
        "--dataset", "-d", default=None,
        help=f"Registry dataset (default: {DEFAULT_DATASET})",
    )
    problems.add_argument(
        "--path", "-p", type=Path, help="Local task or dataset directory"
    )
    parser.add_argument(
        "--include-task", "-i", action="append", default=[],
        help="Task name to include (glob, repeatable)",
    )
    parser.add_argument(
        "--exclude-task", "-x", action="append", default=[],
        help="Task name to exclude (glob, repeatable)",
    )
    parser.add_argument(
        "--attempts", "-k", type=int, default=DEFAULT_ATTEMPTS,
        help=f"Runs per problem (default: {DEFAULT_ATTEMPTS}, the leaderboard requirement)",
    )
    parser.add_argument(
        "--manifest", type=Path, default=DEFAULT_MANIFEST,
        help=f"Team manifest YAML (default: {DEFAULT_MANIFEST.name})",
    )
    parser.add_argument(
        "--endpoint-config", type=Path, default=DEFAULT_ENDPOINTS,
        help=f"Endpoint provider/API-key mapping (default: {DEFAULT_ENDPOINTS.name})",
    )
    parser.add_argument("--n-concurrent", "-n", type=int, default=4, help="Concurrent trials")
    parser.add_argument(
        "--jobs-dir", type=Path, default=PACKAGE_ROOT / "jobs", help="Job output root"
    )
    parser.add_argument("--job-name", default=None, help="Job name (default: lb-<condition>-<UTC>)")
    parser.add_argument(
        "--upload", action="store_true", help="Upload to Harbor Hub when the job finishes"
    )
    parser.add_argument(
        "--gui", action="store_true",
        help="Open the Buzz desktop app as the benchmark user to watch the run live",
    )
    parser.add_argument(
        "--dry-run", action="store_true",
        help="Print the underlying harbor command and exit (no stack bring-up)",
    )
    return parser.parse_args(argv)


# -- state: secrets and identities, generated once --------------------------


def load_state() -> dict[str, str]:
    """Generate-or-load the benchmark environment's keys and secrets."""
    from harbor_buzz_testbed.keys import generate_keypair, keypair_from_secret

    STATE_DIR.mkdir(mode=0o700, exist_ok=True)
    state_path = STATE_DIR / "state.json"
    if state_path.is_file():
        state = json.loads(state_path.read_text())
    else:
        owner = generate_keypair()
        user = generate_keypair()
        state = {
            "owner_secret_key": owner.secret_key,
            "user_secret_key": user.secret_key,
            "postgres_password": secrets.token_urlsafe(24),
            "redis_password": secrets.token_urlsafe(24),
            "typesense_api_key": secrets.token_hex(16),
            "s3_access_key": secrets.token_hex(10),
            "s3_secret_key": secrets.token_hex(20),
            "git_hook_hmac_secret": secrets.token_hex(32),
            "relay_private_key": generate_keypair().secret_key,
        }
        state_path.touch(mode=0o600)
        state_path.write_text(json.dumps(state, indent=2))
    state["owner_pubkey"] = keypair_from_secret(state["owner_secret_key"]).pubkey
    state["user_pubkey"] = keypair_from_secret(state["user_secret_key"]).pubkey
    return state


def write_env_file(state: dict[str, str]) -> Path:
    """Compose interpolation env — regenerated from state on every run."""
    env_path = STATE_DIR / ".env"
    lines = {
        "BUZZ_IMAGE": os.environ.get("BUZZ_IMAGE", "ghcr.io/block/buzz:main"),
        "BUZZ_DOMAIN": "localhost",
        "RELAY_URL": f"ws://localhost:{RELAY_HTTP_PORT}",
        "BUZZ_MEDIA_BASE_URL": f"http://localhost:{RELAY_HTTP_PORT}/media",
        "BUZZ_MEDIA_SERVER_DOMAIN": "localhost",
        "BUZZ_CORS_ORIGINS": f"http://localhost:{RELAY_HTTP_PORT}",
        "BUZZ_REQUIRE_AUTH_TOKEN": "true",
        "BUZZ_REQUIRE_RELAY_MEMBERSHIP": "true",
        "BUZZ_ALLOW_NIP_OA_AUTH": "true",
        "BUZZ_AUTO_MIGRATE": "true",
        "BUZZ_GIT_CONFORMANCE_PROBE": "true",
        "RUST_LOG": "buzz_relay=info,buzz_db=info,buzz_auth=info",
        "RELAY_OWNER_PUBKEY": state["owner_pubkey"],
        "BUZZ_RELAY_PRIVATE_KEY": state["relay_private_key"],
        "BUZZ_GIT_HOOK_HMAC_SECRET": state["git_hook_hmac_secret"],
        "POSTGRES_DB": "buzz",
        "POSTGRES_USER": "buzz",
        "POSTGRES_PASSWORD": state["postgres_password"],
        "REDIS_PASSWORD": state["redis_password"],
        "TYPESENSE_API_KEY": state["typesense_api_key"],
        "BUZZ_S3_ACCESS_KEY": state["s3_access_key"],
        "BUZZ_S3_SECRET_KEY": state["s3_secret_key"],
        "BUZZ_S3_BUCKET": "buzz-media",
        "BUZZ_HTTP_PORT": str(RELAY_HTTP_PORT),
        "BUZZ_PG_HOST_PORT": str(PG_HOST_PORT),
        "BUZZ_METRICS_HOST_PORT": str(METRICS_HOST_PORT),
    }
    env_path.touch(mode=0o600)
    env_path.write_text("".join(f"{k}={v}\n" for k, v in lines.items()))
    return env_path


def postgres_dsn(state: dict[str, str]) -> str:
    return (
        f"postgresql://buzz:{state['postgres_password']}"
        f"@127.0.0.1:{PG_HOST_PORT}/buzz"
    )


def write_provisioner_config(
    state: dict[str, str], endpoint_config: Path
) -> Path:
    """Resolve per-endpoint API keys from the environment and write the
    provisioner config: pinned user, keep-channels teardown."""
    endpoints = json.loads(endpoint_config.read_text())
    llm_api_keys: dict[str, str] = {}
    for name, entry in endpoints.items():
        env_var = entry["api_key_env"]
        key = os.environ.get(env_var)
        if not key:
            raise SystemExit(
                f"endpoint {name!r} needs the {env_var} environment variable"
            )
        llm_api_keys[name] = key
    config = {
        "relay_http_url": f"http://localhost:{RELAY_HTTP_PORT}",
        "relay_ws_url": f"ws://localhost:{RELAY_HTTP_PORT}",
        "owner_secret_key": state["owner_secret_key"],
        "postgres_dsn": postgres_dsn(state),
        "llm_api_keys": llm_api_keys,
        "user_secret_key": state["user_secret_key"],
        "archive_on_teardown": False,
    }
    path = STATE_DIR / "provisioner.json"
    path.touch(mode=0o600)
    path.write_text(json.dumps(config, indent=2))
    return path


# -- docker stack ------------------------------------------------------------


def compose_command(*args: str) -> list[str]:
    command = [
        "docker", "compose",
        "--project-name", COMPOSE_PROJECT,
        "--project-directory", str(STATE_DIR),
        "--env-file", str(STATE_DIR / ".env"),
    ]
    for file in COMPOSE_FILES:
        command += ["-f", str(file)]
    return command + list(args)


def ensure_stack(state: dict[str, str]) -> None:
    """Bring the compose stack up (idempotent) and apply the benchmark schema."""
    subprocess.run(compose_command("up", "-d", "--wait"), check=True)
    import psycopg

    deadline = time.monotonic() + 60
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        try:
            with psycopg.connect(postgres_dsn(state)) as conn:
                conn.execute(SCHEMA_SQL.read_text())
                conn.commit()
            return
        except psycopg.Error as error:  # containers healthy but PG settling
            last_error = error
            time.sleep(2)
    raise SystemExit(f"benchmark schema apply failed: {last_error}")


# -- buzz binaries -----------------------------------------------------------


def ensure_binaries() -> dict[str, Path]:
    """Find the pinned buzz binaries, building them once if missing."""
    try:
        return run_leaderboard.find_binaries(None)
    except SystemExit:
        print("buzz binaries missing — building (cargo build, first run only)...")
    cargo = REPO_ROOT / "bin" / "cargo"
    subprocess.run(
        [str(cargo), "build", "-p", "buzz-cli", "-p", "buzz-acp", "-p", "buzz-agent"],
        cwd=REPO_ROOT,
        check=True,
    )
    return run_leaderboard.find_binaries(None)


# -- GUI ---------------------------------------------------------------------


def launch_gui(state: dict[str, str]) -> subprocess.Popen:
    """Open the Buzz desktop app logged in as the benchmark user.

    The relay runs closed (membership required), so the user pubkey is first
    added to the relay membership list via buzz-admin inside the container —
    NIP-OA auth tags cover the agents, but the GUI authenticates as a plain
    member, exactly like a human.
    """
    subprocess.run(
        compose_command(
            "exec", "-T", "relay",
            "buzz-admin", "add-member", "--pubkey", state["user_pubkey"],
        ),
        check=True,
    )

    desktop_dir = REPO_ROOT / "desktop"
    if not (desktop_dir / "node_modules").is_dir():
        subprocess.run(["pnpm", "install"], cwd=desktop_dir, check=True)

    # tauri dev needs sidecar files present; stub them and drop in the real
    # CLI binary (mirrors the just staging recipe).
    target = subprocess.run(
        ["rustc", "-vV"], capture_output=True, text=True, check=True
    ).stdout
    triple = next(
        line.split(": ", 1)[1] for line in target.splitlines() if line.startswith("host: ")
    )
    sidecar_dir = desktop_dir / "src-tauri" / "binaries"
    sidecar_dir.mkdir(parents=True, exist_ok=True)
    binaries = ensure_binaries()
    for name in ("buzz-acp", "buzz-agent", "buzz-dev-mcp", "git-credential-nostr", "buzz"):
        stub = sidecar_dir / f"{name}-{triple}"
        if not stub.exists():
            stub.touch()
    real_cli = sidecar_dir / f"buzz-{triple}"
    real_cli.write_bytes(binaries["buzz"].read_bytes())
    real_cli.chmod(0o755)

    print(
        f"Opening Buzz GUI as the benchmark user ({state['user_pubkey'][:16]}…).\n"
        "Watch, don't type — a message from you mid-trial would taint the run."
    )
    return subprocess.Popen(
        ["pnpm", "exec", "tauri", "dev"],
        cwd=desktop_dir,
        env={
            **os.environ,
            "BUZZ_RELAY_URL": f"ws://localhost:{RELAY_HTTP_PORT}",
            "BUZZ_PRIVATE_KEY": state["user_secret_key"],
        },
    )


# -- main ---------------------------------------------------------------------


def leaderboard_argv(args: argparse.Namespace, provisioner_config: Path) -> list[str]:
    argv: list[str] = []
    if args.path:
        argv += ["--path", str(args.path)]
    else:
        argv += ["--dataset", args.dataset or DEFAULT_DATASET]
    for pattern in args.include_task:
        argv += ["--include-task", pattern]
    for pattern in args.exclude_task:
        argv += ["--exclude-task", pattern]
    argv += [
        "--attempts", str(args.attempts),
        "--manifest", str(args.manifest),
        "--endpoint-config", str(args.endpoint_config),
        "--provisioner-config", str(provisioner_config),
        "--n-concurrent", str(args.n_concurrent),
        "--jobs-dir", str(args.jobs_dir),
    ]
    if args.job_name:
        argv += ["--job-name", args.job_name]
    if args.upload:
        argv.append("--upload")
    if args.dry_run:
        argv.append("--dry-run")
    return argv


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    state = load_state()
    write_env_file(state)
    provisioner_config = write_provisioner_config(state, args.endpoint_config)

    if not args.dry_run:
        ensure_binaries()
        ensure_stack(state)
        if args.gui:
            launch_gui(state)

    return run_leaderboard.main(leaderboard_argv(args, provisioner_config))


if __name__ == "__main__":
    sys.exit(main())
