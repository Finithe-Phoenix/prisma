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
        raw = json.loads(path.read_text())
        if raw.get("schema_version") != "prisma-bench-result/v1":
            continue
        records.append(RunResult.model_validate(raw))
    return records


def aggregate_results(records: list[RunResult]) -> Summary:
    grouped: dict[tuple[str, BackendName], list[RunResult]] = defaultdict(list)
    for record in records:
        grouped[(record.benchmark, record.backend)].append(record)

    summaries: list[BackendSummary] = []
    for (benchmark, backend), items in sorted(grouped.items()):
        successful = [
            item for item in items
            if not item.skipped and item.exit_code == 0 and item.wall_s is not None
        ]
        wall_times = [item.wall_s for item in successful if item.wall_s is not None]
        ips_values = [
            item.metrics["iterations_per_second"]
            for item in successful
            if "iterations_per_second" in item.metrics
        ]
        summaries.append(
            BackendSummary(
                benchmark=benchmark,
                backend=backend,
                runs=len(items),
                successful_runs=len(successful),
                skipped_runs=sum(1 for item in items if item.skipped),
                best_wall_s=min(wall_times) if wall_times else None,
                median_wall_s=statistics.median(wall_times) if wall_times else None,
                best_iterations_per_second=max(ips_values) if ips_values else None,
            )
        )

    return Summary(records=len(records), summaries=summaries)


def write_summary(results_dir: Path) -> Path:
    records = load_results(results_dir)
    summary = aggregate_results(records)
    out = results_dir / "summary.json"
    out.write_text(summary.to_json_text())
    return out
