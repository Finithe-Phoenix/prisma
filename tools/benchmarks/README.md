# tools/benchmarks — Prisma benchmark harness

**Status:** first runnable slice. `prisma-bench run` can compile and run the
self-authored Dhrystone-style corpus under local backends, emit per-run JSON,
and aggregate `summary.json`. Backends that are unavailable are recorded as
skipped artifacts instead of crashing.

## Purpose

Run a fixed corpus of x86_64 binaries through:

- **Prisma** (at a given commit) — the whole point.
- **QEMU user-mode** — baseline comparison.
- **Box64** — the dynarec baseline on ARM64.
- **FEX** — the IR-based DBT baseline.
- **Native execution** on an x86_64 box — upper bound.

…and record:

- Wall-clock runtime.
- Peak memory.
- Instruction count (if available via perf/papi).
- Translation overhead (time spent in JIT vs time spent in guest code).
- Per-region profiles for TSO-adaptive classification (Pilar 3) and
  hot-path prediction (Pilar 1).

## Why Python + uv

- Fast iteration on result post-processing, table generation, paper
  figures.
- `uv` for dependency management (matches the rest of `tools/`).
- Not on the hot path — the benchmark runner launches native binaries
  and collects output; Python is the scheduler, not the workload.

## Planned corpus

- **Microbenchmarks:** Dhrystone, CoreMark, nbench.
- **SPEC CPU2017 subset** — the ones that run without GUI / network.
- **Per-Pillar micros:** custom binaries that stress TSO-sensitive
  regions, NPU-friendly patterns, graphics translation edge cases.
- **Game-representative workloads:** small kernels extracted from open-
  source game code (Quake, Xash3D, etc.) that mimic hot inner loops.

The first bundled corpus is `dhrystone`, implemented as a self-authored
Dhrystone-style integer workload in `corpora/dhrystone/dhry_prisma.c`. It is
not a verbatim copy of the historic Dhrystone source.

## Usage

```
$ uv venv
$ uv pip install -e '.[dev]'
$ prisma-bench list-corpora
$ prisma-bench run --backend native --corpus dhrystone --output results/
$ prisma-bench run --backend qemu --corpus dhrystone --output results/
$ prisma-bench report results/ --format json
```

`--backend prisma` is wired into the same schema, but currently soft-skips for
the Dhrystone corpus because `core/build/prisma_run` accepts raw blobs rather
than ELF/host binaries. That skip is intentional and visible in the JSON
artifact; it will flip to execution when the runner grows ELF support or a raw
benchmark corpus lands.

## Artifacts

Each `run` writes one JSON file:

```
results/dhrystone-native.json
```

The artifact schema is `prisma-bench-result/v1`. `report --format json` reads
all result artifacts in the directory and writes:

```
results/summary.json
```

Markdown and LaTeX report generation are tracked separately under F2-BM-007.
