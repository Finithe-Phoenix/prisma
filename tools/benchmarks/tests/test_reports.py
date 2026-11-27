from __future__ import annotations

import json
from pathlib import Path

import pytest
from click.testing import CliRunner

from prisma_bench.aggregate import aggregate_results
from prisma_bench.cli import main
from prisma_bench.schema import RunResult


def sample() -> RunResult:
    return RunResult(benchmark="corpus_1", backend="native", host="cpu & os",
                     git_sha="abc", source="a.c", iterations=1, exit_code=0,
                     wall_s=2.0, metrics={"iterations_per_second": 0.5})


@pytest.mark.parametrize(("fmt", "filename"), [
    ("markdown", "summary.md"), ("latex", "summary.tex"), ("json", "summary.json"),
])
def test_reports_include_rows_and_handle_special_characters(
    tmp_path: Path, fmt: str, filename: str,
) -> None:
    (tmp_path / "a.json").write_text(sample().to_json_text())
    result = CliRunner().invoke(main, ["report", str(tmp_path), "--format", fmt])
    assert result.exit_code == 0, result.output
    output = (tmp_path / filename).read_text()
    if fmt == "latex":
        assert r"corpus\_1 & native & cpu \& os" in output
    elif fmt == "markdown":
        assert "| corpus_1 | native | cpu & os |" in output
    else:
        summary = json.loads(output)
        assert summary["records"] == 1
        assert summary["summaries"][0]["median_iterations_per_second"] == 0.5


@pytest.mark.parametrize("content", [
    "not JSON", "[]", '{"schema_version":"prisma-bench-result/v1"}',
])
def test_bad_artifact_identifies_file_and_preserves_report(tmp_path: Path, content: str) -> None:
    (tmp_path / "broken.json").write_text(content)
    previous = tmp_path / "summary.json"
    previous.write_text("previous report")
    result = CliRunner().invoke(main, ["report", str(tmp_path), "--format", "json"])
    assert result.exit_code != 0
    assert "broken.json" in result.output
    assert previous.read_text() == "previous report"


def test_aggregate_partitions_hosts_and_excludes_validation_failures() -> None:
    first = sample()
    second = first.model_copy(update={"metrics": {"iterations_per_second": 1.5}})
    invalid = first.model_copy(update={"validation_error": "wrong checksum"})
    other_host = first.model_copy(update={"host": "z"})
    rows = aggregate_results([first, second, invalid, other_host]).summaries
    assert len(rows) == 2
    assert rows[0].successful_runs == 2
    assert rows[0].failed_runs == 1
    assert rows[0].median_iterations_per_second == 1
    assert rows[0].mad_iterations_per_second == 0.5
