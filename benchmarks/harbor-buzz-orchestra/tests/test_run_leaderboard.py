"""The leaderboard runner must emit only leaderboard-legal harbor settings."""

import importlib.util
import json
import sys
from pathlib import Path

import pytest
import yaml

_SCRIPT = Path(__file__).parent.parent / "scripts" / "run_leaderboard.py"
_spec = importlib.util.spec_from_file_location("run_leaderboard", _SCRIPT)
run_leaderboard = importlib.util.module_from_spec(_spec)
sys.modules["run_leaderboard"] = run_leaderboard
_spec.loader.exec_module(run_leaderboard)

FORBIDDEN_FLAGS = (
    "--timeout-multiplier",
    "--agent-timeout-multiplier",
    "--verifier-timeout-multiplier",
    "--agent-setup-timeout-multiplier",
    "--environment-build-timeout-multiplier",
    "--override-cpus",
    "--override-memory",
    "--override-storage",
    "--override-gpus",
)


@pytest.fixture
def binaries(tmp_path):
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    found = {}
    for name in run_leaderboard.BINARIES:
        path = bin_dir / name
        path.write_text("#!/bin/sh\n")
        found[name] = path
    return found


@pytest.fixture
def args(tmp_path, binaries):
    manifest = tmp_path / "team.yaml"
    manifest.write_text(
        yaml.safe_dump(
            {
                "condition": "unit-test",
                "roster": [
                    {"id": "orch", "endpoint": "frontier"},
                    {"id": "worker", "endpoint": "fast", "count": 2},
                ],
            }
        )
    )
    endpoints = tmp_path / "endpoints.json"
    endpoints.write_text(json.dumps({"frontier": {"provider": "anthropic"}}))
    provisioner = tmp_path / "provisioner.json"
    provisioner.write_text("{}")
    return run_leaderboard.parse_args(
        [
            "--dataset",
            "terminal-bench/terminal-bench-2-1",
            "--attempts",
            "5",
            "--manifest",
            str(manifest),
            "--endpoint-config",
            str(endpoints),
            "--provisioner-config",
            str(provisioner),
            "--job-name",
            "unit-test-job",
        ]
    )


def test_command_uses_standard_settings_only(args, binaries):
    command = run_leaderboard.build_command(args, binaries)
    assert command[:2] == ["harbor", "run"]
    assert command.count("-k") == 1
    assert command[command.index("-k") + 1] == "5"
    for flag in FORBIDDEN_FLAGS:
        assert flag not in command


def test_forbidden_flags_are_not_accepted():
    for flag in FORBIDDEN_FLAGS:
        with pytest.raises(SystemExit):
            run_leaderboard.parse_args(
                ["--dataset", "d", "--attempts", "5", flag, "1"]
            )


def test_metadata_template_matches_harbor_schema(args, tmp_path):
    from harbor.leaderboard.metadata import load_metadata

    path = run_leaderboard.write_metadata_template(args, tmp_path)
    loaded = load_metadata(path)
    assert loaded["agent_org_display_name"] == "Block"
    assert [m["model_name"] for m in loaded["models"]] == ["frontier", "fast"]
    assert loaded["models"][0]["model_org_display_name"] == "Anthropic"
    assert loaded["models"][1]["model_provider"] == "FILL_ME"
