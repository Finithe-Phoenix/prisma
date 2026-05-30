from __future__ import annotations

import pytest
from pydantic import ValidationError

from prisma_bench.schema import RunResult


def test_run_result_requires_skip_reason_when_skipped() -> None:
    with pytest.raises(ValidationError):
        RunResult(
            benchmark="dhrystone",
            backend="prisma",
            host="test",
            git_sha="abc123",
            source="dhry_prisma.c",
            iterations=1,
            skipped=True,
        )


def test_run_result_accepts_successful_artifact() -> None:
    result = RunResult(
        benchmark="dhrystone",
        backend="native",
        host="test",
        git_sha="abc123",
        command=["./dhry_prisma", "1"],
        source="dhry_prisma.c",
        binary="./dhry_prisma",
        iterations=1,
        exit_code=0,
        wall_s=0.01,
        metrics={"iterations_per_second": 100.0},
    )
    assert result.schema_version == "prisma-bench-result/v1"
    assert "iterations_per_second" in result.to_json_text()
