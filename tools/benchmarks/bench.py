#!/usr/bin/env python3
"""F1-AC-003 — baseline benchmark driver for Prisma.

Runs the same x86_64 benchmark binary through every available
emulator on this host and emits a JSON record:

  * `prisma`    — our DBT (`Translator::translate` + run, via the
                  helper binary in `core/build/prisma_run`; gated
                  on that binary existing).
  * `qemu`      — qemu-x86_64 user-mode (`brew install qemu` on
                  macOS does NOT supply this; install via Linux
                  distro package or build from source).
  * `box64`     — Box64 if `box64` is on PATH.
  * `fex`       — FEX-Emu if `FEXLoader` is on PATH.
  * `native`    — direct exec on hosts that can run the binary
                  natively (an x86 host running an x86 binary).

Each backend that's missing is recorded as `"skipped"` rather than
crashing. The driver is designed for CI on the future
`prisma-linux-arm64` runner (F0-DX-014) and for local triage on
the dev box.

Output schema (one JSON object per run):

  {
    "binary":      "path/to/dhrystone",
    "host":        "Darwin arm64 / Apple M3 Max",
    "git_sha":     "abc1234",
    "timestamp":   "2026-05-04T18:30:00Z",
    "results": [
      { "engine": "qemu",    "exit": 0, "wall_s": 0.42, "skipped": false },
      { "engine": "box64",   "exit": 0, "wall_s": 0.18, "skipped": false },
      { "engine": "fex",     "skipped": true,
                             "reason": "FEXLoader not on PATH" },
      { "engine": "prisma",  "skipped": true,
                             "reason": "core/build/prisma_run not built" },
      { "engine": "native",  "skipped": true,
                             "reason": "host arch does not match binary" }
    ]
  }

Run with `--engines qemu,box64` to restrict the engine set.
"""

from __future__ import annotations

import argparse
import datetime
import json
import os
import platform
import shutil
import subprocess
import sys
import time
from collections.abc import Callable
from dataclasses import asdict, dataclass
from pathlib import Path


# Engine probe: returns the executable path or None.
def _find(*candidates: str) -> str | None:
    for c in candidates:
        p = shutil.which(c)
        if p is not None:
            return p
    return None


@dataclass
class EngineResult:
    engine: str
    skipped: bool = False
    reason: str = ""
    exit: int | None = None
    wall_s: float | None = None
    stdout_tail: str = ""
    stderr_tail: str = ""


def _run_with(engine: str, argv: list[str], timeout_s: float = 60.0) -> EngineResult:
    t0 = time.monotonic()
    try:
        p = subprocess.run(argv, capture_output=True, timeout=timeout_s)
    except subprocess.TimeoutExpired as exc:
        return EngineResult(engine=engine, skipped=False, exit=-1,
                            wall_s=timeout_s,
                            reason=f"timeout after {timeout_s}s",
                            stderr_tail=str(exc.stderr or "")[:200])
    wall = time.monotonic() - t0
    return EngineResult(
        engine=engine, skipped=False,
        exit=p.returncode, wall_s=wall,
        stdout_tail=p.stdout[-200:].decode(errors="replace"),
        stderr_tail=p.stderr[-200:].decode(errors="replace"),
    )


def run_qemu(binary: Path) -> EngineResult:
    qemu = _find("qemu-x86_64", "qemu-x86_64-static")
    if qemu is None:
        return EngineResult("qemu", skipped=True,
                            reason="qemu-x86_64 not on PATH")
    return _run_with("qemu", [qemu, str(binary)])


def run_box64(binary: Path) -> EngineResult:
    box64 = _find("box64")
    if box64 is None:
        return EngineResult("box64", skipped=True,
                            reason="box64 not on PATH")
    return _run_with("box64", [box64, str(binary)])


def run_fex(binary: Path) -> EngineResult:
    fex = _find("FEXLoader", "FEXInterpreter")
    if fex is None:
        return EngineResult("fex", skipped=True,
                            reason="FEXLoader not on PATH")
    return _run_with("fex", [fex, str(binary)])


def run_native(binary: Path) -> EngineResult:
    # Best-effort: only if the host arch is x86_64.
    if platform.machine() not in ("x86_64", "AMD64"):
        return EngineResult("native", skipped=True,
                            reason=f"host arch {platform.machine()} != x86_64")
    if not os.access(binary, os.X_OK):
        return EngineResult("native", skipped=True,
                            reason="binary not marked executable")
    return _run_with("native", [str(binary)])


def run_prisma(binary: Path, repo_root: Path) -> EngineResult:
    runner = repo_root / "core" / "build" / "prisma_run"
    if not runner.exists():
        return EngineResult("prisma", skipped=True,
                            reason=f"{runner.relative_to(repo_root)} not built; "
                                   "build with `cmake --build core/build "
                                   "--target prisma_run` once the runner "
                                   "binary lands")
    return _run_with("prisma", [str(runner), str(binary)])


_ENGINES: dict[str, Callable[[Path, Path], EngineResult]] = {
    "qemu":   lambda b, _: run_qemu(b),
    "box64":  lambda b, _: run_box64(b),
    "fex":    lambda b, _: run_fex(b),
    "native": lambda b, _: run_native(b),
    "prisma": lambda b, root: run_prisma(b, root),
}


def host_label() -> str:
    return f"{platform.system()} {platform.machine()} / {platform.processor() or 'unknown'}"


def git_sha(repo_root: Path) -> str:
    try:
        out = subprocess.run(
            ["git", "-C", str(repo_root), "rev-parse", "--short", "HEAD"],
            capture_output=True, text=True, check=True,
        )
        return out.stdout.strip()
    except subprocess.CalledProcessError:
        return "unknown"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("binary", type=Path,
                        help="x86_64 ELF binary to benchmark")
    parser.add_argument("--engines", default="qemu,box64,fex,prisma,native",
                        help="comma-separated engine list (default: all)")
    parser.add_argument("--json", type=Path, default=None,
                        help="write the JSON record to this file too")
    args = parser.parse_args()

    if not args.binary.exists():
        print(f"error: {args.binary} does not exist", file=sys.stderr)
        return 1

    here = Path(__file__).resolve().parent
    repo_root = here.parent.parent  # tools/benchmarks/ → repo root.

    requested = [e.strip() for e in args.engines.split(",") if e.strip()]
    unknown = [e for e in requested if e not in _ENGINES]
    if unknown:
        print(f"error: unknown engine(s): {unknown}", file=sys.stderr)
        return 1

    record = {
        "binary":    str(args.binary),
        "host":      host_label(),
        "git_sha":   git_sha(repo_root),
        "timestamp": datetime.datetime.now(datetime.UTC).isoformat(timespec="seconds"),
        "results":   [],
    }

    for engine in requested:
        r = _ENGINES[engine](args.binary, repo_root)
        record["results"].append({k: v for k, v in asdict(r).items() if v != ""})

    out = json.dumps(record, indent=2)
    print(out)
    if args.json is not None:
        args.json.write_text(out + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
