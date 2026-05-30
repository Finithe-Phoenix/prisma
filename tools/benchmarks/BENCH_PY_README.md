# `tools/benchmarks/bench.py` — F1-AC-003 immediate-runnable baseline driver

A small standalone Python driver that runs an x86_64 binary through every
emulator on this host and emits JSON. Pure stdlib (no `uv`, no `click`).

For the richer corpus-aware CLI, see `src/prisma_bench/` and the main
`README.md` here. The two coexist: `bench.py` is the "ssh in, run a binary,
see numbers" fallback that works on any host with Python 3.10+, while
`prisma-bench` builds known corpora and emits versioned result artifacts.

## Quick start

```sh
python3 tools/benchmarks/bench.py path/to/dhrystone
python3 tools/benchmarks/bench.py --engines qemu,box64 path/to/coremark
python3 tools/benchmarks/bench.py --json out.json path/to/binary
```

## Engines probed

| Engine   | How probed                            | Install hint               |
|----------|---------------------------------------|----------------------------|
| `qemu`   | `qemu-x86_64` / `qemu-x86_64-static`  | Linux: `apt install qemu-user`; macOS: build from source |
| `box64`  | `box64`                                | Build from source — no macOS bottle yet |
| `fex`    | `FEXLoader` / `FEXInterpreter`         | https://github.com/FEX-Emu/FEX |
| `prisma` | `core/build/prisma_run`                | Built when the runner target lands |
| `native` | `platform.machine() == x86_64`         | Only fires on x86 hosts |

Engines absent from `PATH` (or on the wrong arch) are recorded as
`{ "skipped": true, "reason": "..." }` rather than crashing — the
driver soft-passes on every host.
