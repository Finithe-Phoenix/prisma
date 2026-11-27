# Prisma benchmark harness

`prisma-bench` builds the selected corpus, runs it with the requested backend,
and writes a versioned JSON record with the exit status, command, binary digest,
host, kernel, Git revision, output and measurements. The summary groups samples
by workload, backend, commit, host, target architecture and iteration count.

## Reproduce a baseline

From the repository root, use Python 3.12+ and a C compiler. The CoreMark and
nbench build scripts use Linux x86_64, GNU Make, curl, sha256sum and the
x86_64 GNU toolchain with static libraries. The first build downloads and
verifies the pinned sources. Later builds reuse the verified archives in the
output directory.

```sh
python -m pip install -e 'tools/benchmarks[dev]'
prisma-bench list-corpora
prisma-bench run --backend native --corpus dhrystone --iterations 1000 --output smoke/
for sample in 1 2 3; do
  prisma-bench run --backend native --corpus coremark --sample "$sample" --timeout-s 300 --output results/
done
prisma-bench report results/ --format json
prisma-bench report results/ --format markdown
prisma-bench report results/ --format latex
```

Supply a fresh output directory for each experiment. The sample number identifies
the individual result file, and repeating the same number replaces that sample.
`report` writes `summary.json`, `summary.md` or `summary.tex`. Malformed inputs
produce a diagnostic naming the file.

| Corpus | Workload and result |
| --- | --- |
| dhrystone | Self-authored integer workload. The 1,000-iteration smoke run checks a fixed checksum. Larger runs measure iterations per wall-clock second. |
| coremark | Pinned CoreMark performance seeds. The upstream CRC and minimum duration checks must pass. Increase iterations on faster machines to meet CoreMark's ten-second minimum. |
| nbench | Numeric-sort profile from BYTEmark 2.2.3. The score uses nbench's reported iterations per second. Each sample performs one full invocation of this profile. |

The benchmark workflow checks Dhrystone on pushes and pull requests. Use
**Actions → benchmarks → Run workflow** for three samples of every corpus on
native x86_64 and QEMU and for records of other backends available to the job.
The raw-blob Prisma runner currently requires an ELF execution path before it
can supply scores for these corpora. Missing backends appear as skipped in the
reports. Compiler errors, failed workloads and checksum mismatches fail the run.

## Interpreting regressions

Correctness failures block the smoke job. For performance triage, compare
like-for-like batches on the same CPU, OS, toolchain and workload. Start with
the median throughput and median absolute deviation (MAD) in the JSON summaries.
Treat a slowdown larger than both 5% and three times the larger batch MAD
as a candidate regression. Repeat the pair of batches on the same machine and
require the slowdown in both comparisons before filing a performance regression.
CI publishes timing observations for review. Recalibrate the advisory threshold
with recorded variation before enabling any automated timing gate.

The standalone driver for prebuilt binaries is documented in
[BENCH_PY_README.md](BENCH_PY_README.md).
