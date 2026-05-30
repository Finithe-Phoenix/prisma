# `tools/diff-qemu/` — F1-TC-003 differential decoder harness

Cross-checks Prisma's translation against `qemu-x86_64` (user-mode).

## Quick start

```sh
python3 tools/diff-qemu/run.py --probe
```

If `qemu-x86_64` is on your PATH, the probe builds a 132-byte ELF that
returns 42, runs it through QEMU, and asserts QEMU sees exit code 42.

If not on PATH the script exits 0 with a clear "SKIP" message and
install hints for Linux and macOS — the script is designed to be a
soft-skip in CI on hosts without QEMU user-mode.

## Why a Python script and not a CMake test

- QEMU user-mode (`qemu-x86_64`) is **not** on Homebrew for macOS
  (only `qemu-system-*`). The CI Linux runner (`prisma-linux-arm64`,
  per `F0-DX-014`) has it; local macOS dev installs via:
  - `brew install qemu` won't help — the user-mode targets are
    not built in the bottle.
  - Docker Desktop's `linux/amd64` Rosetta layer silently fails
    on non-trivial binaries today (we tried; emulated bash
    "cannot execute binary file"); bring this back when Apple
    ships a 16K-page-friendly amd64 path.
  - Build qemu from source with `--enable-linux-user` (~5 min).
- ELF generation is more naturally Python than CMake.

## Roadmap

- **Now**: probe — qemu is alive and reports exit codes.
- **Next**: a `--corpus PATH` mode that takes a file of byte
  sequences (one per line, hex-encoded) and runs each through
  qemu, recording final RAX into a JSON record.
- **Later**: the matching mode in `core/`'s Translator that
  records final RAX after a single Translator invocation. The
  diff is then a JSON-vs-JSON compare.
- **Eventually**: extend to a full coreutils corpus (target ≥99%
  agreement per F1-DC-087).
