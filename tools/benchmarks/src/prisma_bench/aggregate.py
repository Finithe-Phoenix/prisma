"""Aggregate benchmark JSON artifacts."""

from __future__ import annotations

import json
import statistics
from collections import defaultdict
from pathlib import Path

from .schema import BackendName, BackendSummary, RunResult, Summary


def load_results(results_dir: Path) -> list[RunResult]:
    records: list[RunResult] = []
    for path in sorted(results_dir.glob("*.json")):
        if path.name == "summary.json":
            continue
        try:
            raw = json.loads(path.read_text(encoding="utf-8"))
            if not isinstance(raw, dict):
                raise ValueError("expected a JSON object")
            if raw.get("schema_version") != "prisma-bench-result/v1":
                continue
            records.append(RunResult.model_validate(raw))
        except (OSError, ValueError) as exc:
            raise ValueError(f"{path.name}: {exc}") from exc
    return records


def aggregate_results(records: list[RunResult]) -> Summary:
    grouped: dict[tuple[str, BackendName, str, str, int, str], list[RunResult]] = defaultdict(list)
    for record in records:
        key = (record.benchmark, record.backend, record.host, record.git_sha,
               record.iterations, record.target_arch or "")
        grouped[key].append(record)

    summaries: list[BackendSummary] = []
    for (benchmark, backend, host, sha, iterations, arch), items in sorted(grouped.items()):
        successful = [
            item for item in items
            if not item.skipped and item.exit_code == 0 and item.wall_s is not None
            and item.validation_error is None
        ]
        wall_times = [item.wall_s for item in successful if item.wall_s is not None]
        ips_values = [
            item.metrics["iterations_per_second"]
            for item in successful
            if "iterations_per_second" in item.metrics
        ]
        median_ips = statistics.median(ips_values) if ips_values else None
        summaries.append(
            BackendSummary(
                benchmark=benchmark,
                backend=backend,
                host=host,
                git_sha=sha,
                iterations=iterations,
                target_arch=arch or None,
                runs=len(items),
                failed_runs=sum(1 for item in items if not item.skipped) - len(successful),
                successful_runs=len(successful),
                skipped_runs=sum(1 for item in items if item.skipped),
                best_wall_s=min(wall_times) if wall_times else None,
                median_wall_s=statistics.median(wall_times) if wall_times else None,
                best_iterations_per_second=max(ips_values) if ips_values else None,
                median_iterations_per_second=median_ips,
                mad_iterations_per_second=(
                    statistics.median(abs(ips - median_ips) for ips in ips_values)
                    if median_ips is not None else None
                ),
            )
        )

    return Summary(records=len(records), summaries=summaries)


def write_summary(results_dir: Path) -> Path:
    records = load_results(results_dir)
    summary = aggregate_results(records)
    out = results_dir / "summary.json"
    out.write_text(summary.to_json_text(), encoding="utf-8")
    return out


def write_report(results_dir: Path, fmt: str) -> Path:
    if fmt == "json":
        return write_summary(results_dir)
    summary = aggregate_results(load_results(results_dir))
    rows = [[
        "Corpus", "Backend", "Host", "Commit", "Iterations", "Passed", "Failed", "Skipped",
        "Median wall (s)", "Median iterations/s",
    ]]
    for item in summary.summaries:
        rows.append([
            item.benchmark, item.backend, item.host, item.git_sha, str(item.iterations),
            str(item.successful_runs), str(item.failed_runs), str(item.skipped_runs),
            f"{item.median_wall_s:.6g}" if item.median_wall_s is not None else "N/A",
            f"{item.median_iterations_per_second:.6g}"
            if item.median_iterations_per_second is not None else "N/A",
        ])
    if fmt == "markdown":
        out = results_dir / "summary.md"
        lines = [
            "| " + " | ".join(cell.replace("\n", " ").replace("|", r"\|") for cell in row) + " |"
            for row in rows
        ]
        lines.insert(1, "| " + " | ".join(["---"] * len(rows[0])) + " |")
    elif fmt == "latex":
        out = results_dir / "summary.tex"
        escape = {c: "\\" + c for c in "&%$#_{}"}
        escape.update({"\\": r"\textbackslash{}", "~": r"\textasciitilde{}",
                       "^": r"\textasciicircum{}", "\n": " "})
        lines = [r"\begin{tabular}{" + "l" * len(rows[0]) + "}"]
        for row in rows:
            escaped = ["".join(escape.get(c, c) for c in cell) for cell in row]
            lines.append(" & ".join(escaped) + r" \\")
        lines.append(r"\end{tabular}")
    else:
        raise ValueError(f"unknown report format: {fmt}")
    out.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return out
