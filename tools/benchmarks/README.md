# tools/benchmarks — Prisma benchmark harness

**Status:** scaffolding only. First real benchmark runs are Fase 2+
(semana 49 onwards). This package exists now so the API is clarified
early and CI can wire it in at that time.

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

None of these exist yet. Each has its own schedule tied to the Fase
that needs it.

## Usage (future)

```
$ uv venv
$ uv pip install -e '.[dev]'
$ prisma-bench run --config configs/dhrystone.toml --backend prisma --output results/
$ prisma-bench report results/ --format markdown
```

The CLI is a stub (`src/prisma_bench/cli.py`) that prints a deliberate
"not yet implemented" message so nothing ships by accident. We will fill
it in starting Fase 2.
