"""Smoke tests for the benchmark CLI skeleton.

These assertions pin the surface area so future fills-in cannot
accidentally remove commands users may script against.
"""

from __future__ import annotations

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


def test_run_stub_exits_non_zero_with_message() -> None:
    runner = CliRunner()
    result = runner.invoke(main, ["run", "--backend", "prisma", "--corpus", "dhrystone"])
    assert result.exit_code == 2
    assert "not yet implemented" in result.output
    assert "Fase 2" in result.output


def test_report_stub_exits_non_zero_with_message(tmp_path: object) -> None:
    # tmp_path is a pytest fixture; Click's `exists=True` on the path argument
    # requires it to exist, so we hand in the pytest-provided directory.
    runner = CliRunner()
    result = runner.invoke(main, ["report", str(tmp_path)])
    assert result.exit_code == 2
    assert "not yet implemented" in result.output
