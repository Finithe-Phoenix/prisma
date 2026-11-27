# GNU coreutils execution corpus

EWP-A7 tracks execution of Linux x86-64 utilities. This first corpus runs
31 distinct binaries with fixed arguments and stdin. It covers text filters,
hashing, encodings, arithmetic, predicates and paths. The manifest is
[cases.json](cases.json). The wider GNU upstream test suite, file mutations,
locales other than C, and workload-dependent threading remain future coverage.

## Capture and compare

Use Python 3.11 or later. Capture the native oracle on Linux x86_64 from the
GNU coreutils binaries selected by `--bin-dir` (default `/usr/bin`). Copy the
same x86-64 binaries, their ELF loader, libraries and native report when moving
the comparison to another host.

```sh
python3 tools/coreutils/run.py --backend native --output results/native.json
python3 tools/coreutils/run.py --backend qemu --baseline results/native.json --output results/qemu.json
```

Supply a Prisma runner that accepts an ELF followed by the utility arguments.
For a runner that accepts extra options, pass each option with `--runner-arg`.
QEMU can use a separate sysroot with `--runner-arg=-L --runner-arg=/path/to/sysroot`.

```sh
python3 tools/coreutils/run.py --backend prisma --runner /path/to/prisma-elf-runner \
  --bin-dir /path/to/x86_64/coreutils --baseline results/native.json --output results/prisma.json
```

## Scoring and reproducibility

The denominator is every manifest case, including missing binaries and differing
binary hashes. A pass requires the expected exit code and byte-for-byte agreement
with native stdout and stderr. A missing backend or incompatible baseline is
an invocation error. A comparison exits unsuccessfully below 85%. Each report
records the actual host and kernel, runner command and digest, manifest digest,
per-binary SHA-256, outputs and disposition for every case.

| Reference observation | Host | Cases passed | Failed | Skipped |
| --- | --- | ---: | ---: | ---: |
| Native GNU coreutils 9.7 | Linux x86_64, kernel 6.18.57 | 31/31 | 0 | 0 |
| QEMU user 10.0.13 vs native | Linux x86_64, kernel 6.18.57 | 31/31 | 0 | 0 |

This table measures the reference harness. Evaluating the Prisma phase target
requires a Prisma ELF runner on ARM64 and the agreed coverage of the GNU suite.
The CI job uploads the full native and QEMU reports whenever the harness changes.
