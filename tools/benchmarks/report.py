#!/usr/bin/env python3
import argparse
import json
import statistics
import sys
from collections import defaultdict


def generate_markdown(data):
    # data is a dict: engine -> benchmark -> list of times
    engines = list(data)
    if not engines:
        return "No data."

    benchmarks = set()
    for engine in engines:
        benchmarks.update(data[engine])
    benchmarks = sorted(benchmarks)

    lines = []
    header = "| Benchmark | " + " | ".join(engines) + " |"
    lines.append(header)
    lines.append("|---| " + " | ".join(["---"] * len(engines)) + " |")

    for benchmark in benchmarks:
        row = [benchmark]
        for engine in engines:
            times = data[engine].get(benchmark, [])
            if not times:
                row.append("N/A")
            else:
                mean = statistics.mean(times)
                if len(times) > 1:
                    std = statistics.stdev(times)
                    row.append(f"{mean:.3f}s (±{std:.3f}s)")
                else:
                    row.append(f"{mean:.3f}s")
        lines.append("| " + " | ".join(row) + " |")
    return "\n".join(lines)


def generate_latex(data):
    engines = list(data)
    if not engines:
        return "No data."

    benchmarks = set()
    for engine in engines:
        benchmarks.update(data[engine])
    benchmarks = sorted(benchmarks)

    lines = ["\\begin{table}[h]", "\\centering"]
    columns = "l" + "c" * len(engines)
    lines.append(f"\\begin{{tabular}}{{{columns}}}")
    lines.append("\\hline")
    latex_line_break = chr(92) * 2
    lines.append("Benchmark & " + " & ".join(engines) + " " + latex_line_break)
    lines.append("\\hline")

    for benchmark in benchmarks:
        row = [benchmark.replace("_", "\\_")]
        for engine in engines:
            times = data[engine].get(benchmark, [])
            if not times:
                row.append("N/A")
            else:
                mean = statistics.mean(times)
                if len(times) > 1:
                    std = statistics.stdev(times)
                    row.append(f"{mean:.3f}s ($\\pm${std:.3f}s)")
                else:
                    row.append(f"{mean:.3f}s")
        lines.append(" & ".join(row) + " " + latex_line_break)

    lines.extend(
        [
            "\\hline",
            "\\end{tabular}",
            "\\caption{Benchmark Results}",
            "\\end{table}",
        ]
    )
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description="Generate benchmark reports")
    parser.add_argument("files", nargs="+", help="JSON result files from bench.py")
    parser.add_argument(
        "--latex",
        action="store_true",
        help="Output LaTeX table instead of Markdown",
    )
    args = parser.parse_args()

    # Expected JSON structure per file:
    # {
    #   "results": [
    #      {"engine": "prisma", "benchmark": "coremark", "time": 1.25},
    #      {"engine": "prisma", "benchmark": "coremark", "time": 1.26},
    #   ]
    # }
    data = defaultdict(lambda: defaultdict(list))

    for file_name in args.files:
        try:
            with open(file_name, encoding="utf-8") as file_pointer:
                payload = json.load(file_pointer)
        except (OSError, json.JSONDecodeError) as exc:
            print(f"Error reading {file_name}: {exc}", file=sys.stderr)
            continue

        for result in payload.get("results", []):
            engine = result.get("engine", "unknown")
            benchmark = result.get("benchmark", "unknown")
            if "time" in result:
                data[engine][benchmark].append(result["time"])
            elif "times" in result:
                data[engine][benchmark].extend(result["times"])

    if args.latex:
        print(generate_latex(data))
    else:
        print(generate_markdown(data))


if __name__ == "__main__":
    main()
