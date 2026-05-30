"""Smoke tests for the benchmark CLI skeleton.

These assertions pin the surface area so future fills-in cannot
accidentally remove commands users may script against.
"""

from __future__ import annotations

import json
from pathlib import Path

from click.testing import CliRunner

from prisma_bench.cli import main


def test_version_flag_exits_zero() -> None:
    runner = CliRunner()
    result = runner.invoke(main, ["--version"])
    assert result.exit_code == 0
    assert "prisma-bench" in result.output


def test_list_corpora_is_implemented() -> None:
    runner = CliRunner()
    result = runner.invoke(main, ["list-corpora"])
    assert result.exit_code == 0
    assert "dhrystone" in result.output


def test_run_prisma_writes_skipped_json(tmp_path: Path) -> None:
    runner = CliRunner()
    result = runner.invoke(
        main,
        [
            "run",
            "--backend",
            "prisma",
            "--corpus",
            "dhrystone",
            "--output",
            str(tmp_path),
        ],
    )
    assert result.exit_code == 0
    artifact = tmp_path / "dhrystone-prisma.json"
    assert artifact.exists()
    raw = json.loads(artifact.read_text())
    assert raw["schema_version"] == "prisma-bench-result/v1"
    assert raw["benchmark"] == "dhrystone"
    assert raw["backend"] == "prisma"
    assert raw["skipped"] is True
    assert raw["skip_reason"]


def test_report_json_writes_summary(tmp_path: Path) -> None:
    runner = CliRunner()
    run_result = runner.invoke(
        main,
        [
            "run",
            "--backend",
            "prisma",
            "--corpus",
            "dhrystone",
            "--output",
            str(tmp_path),
        ],
    )
    assert run_result.exit_code == 0
    report_result = runner.invoke(main, ["report", str(tmp_path), "--format", "json"])
    assert report_result.exit_code == 0
    summary = tmp_path / "summary.json"
    assert summary.exists()
    raw = json.loads(summary.read_text())
    assert raw["schema_version"] == "prisma-bench-summary/v1"
    assert raw["records"] == 1


def test_report_latex_remains_future_work(tmp_path: Path) -> None:
    runner = CliRunner()
    result = runner.invoke(main, ["report", str(tmp_path), "--format", "latex"])
    assert result.exit_code == 2
    assert "not yet implemented" in result.output
