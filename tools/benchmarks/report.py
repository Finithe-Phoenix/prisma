#!/usr/bin/env python3
import argparse
import json
import sys
import statistics
from collections import defaultdict

def generate_markdown(data):
    # data is a dict: engine -> benchmark -> list of times
    engines = list(data.keys())
    if not engines:
        return "No data."
    
    benchmarks = set()
    for e in engines:
        benchmarks.update(data[e].keys())
    benchmarks = sorted(list(benchmarks))
    
    lines = []
    header = "| Benchmark | " + " | ".join(engines) + " |"
    lines.append(header)
    lines.append("|---| " + " | ".join(["---"] * len(engines)) + " |")
    
    for b in benchmarks:
        row = [b]
        for e in engines:
            times = data[e].get(b, [])
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
    engines = list(data.keys())
    if not engines:
        return "No data."
    
    benchmarks = set()
    for e in engines:
        benchmarks.update(data[e].keys())
    benchmarks = sorted(list(benchmarks))
    
    lines = []
    lines.append("\\begin{table}[h]")
    lines.append("\\centering")
    cols = "l" + "c" * len(engines)
    lines.append(f"\\begin{{tabular}}{{{cols}}}")
    lines.append("\\hline")
    header = "Benchmark & " + " & ".join(engines) + " \\\\"
    lines.append(header)
    lines.append("\\hline")
    
    for b in benchmarks:
        row = [b.replace("_", "\\_")]
        for e in engines:
            times = data[e].get(b, [])
            if not times:
                row.append("N/A")
            else:
                mean = statistics.mean(times)
                if len(times) > 1:
                    std = statistics.stdev(times)
                    row.append(f"{mean:.3f}s ($\\pm${std:.3f}s)")
                else:
                    row.append(f"{mean:.3f}s")
        lines.append(" & ".join(row) + " \\\\")
        
    lines.append("\\hline")
    lines.append("\\end{tabular}")
    lines.append("\\caption{Benchmark Results}")
    lines.append("\\end{table}")
    return "\n".join(lines)

def main():
    parser = argparse.ArgumentParser(description="Generate benchmark reports")
    parser.add_argument("files", nargs="+", help="JSON result files from bench.py")
    parser.add_argument("--latex", action="store_true", help="Output LaTeX table instead of Markdown")
    args = parser.parse_args()

    # expected JSON structure per file:
    # {
    #   "results": [
    #      { "engine": "prisma", "benchmark": "coremark", "time": 1.25 },
    #      { "engine": "prisma", "benchmark": "coremark", "time": 1.26 }
    #   ]
    # }
    
    data = defaultdict(lambda: defaultdict(list))
    
    for f in args.files:
        try:
            with open(f, 'r') as fp:
                j = json.load(fp)
                if "results" in j:
                    for r in j["results"]:
                        eng = r.get("engine", "unknown")
                        bm = r.get("benchmark", "unknown")
                        if "time" in r:
                            data[eng][bm].append(r["time"])
                        elif "times" in r:
                            data[eng][bm].extend(r["times"])
                else:
                    # Alternative flat structure or just ignore
                    pass
        except Exception as e:
            print(f"Error reading {f}: {e}", file=sys.stderr)
            
    if args.latex:
        print(generate_latex(data))
    else:
        print(generate_markdown(data))

if __name__ == "__main__":
    main()
