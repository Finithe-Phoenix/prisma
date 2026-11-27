#!/usr/bin/env python3
"""Compare an x86-64 GNU coreutils corpus against a captured native run."""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import platform
import shutil
import struct
import subprocess
import sys
import tempfile
from collections import Counter
from pathlib import Path

CASES = Path(__file__).with_name("cases.json")
SCHEMA = "prisma-coreutils/v1"


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def is_x86_64_elf(binary: Path) -> bool:
    with binary.open("rb") as stream:
        header = stream.read(20)
    return (len(header) == 20 and header[:6] == b"\x7fELF\x02\x01"
            and struct.unpack_from("<H", header, 18)[0] == 62)


def execute(binary, args, stdin, runner, cwd, timeout):
    env = dict(os.environ, LC_ALL="C", LANG="C", TZ="UTC")
    proc = subprocess.run(
        [*runner, str(binary), *args], cwd=cwd, env=env, input=stdin.encode(),
        check=False, capture_output=True, timeout=timeout,
    )
    return {
        "exit": proc.returncode,
        "stdout_b64": base64.b64encode(proc.stdout).decode("ascii"),
        "stderr_b64": base64.b64encode(proc.stderr).decode("ascii"),
    }


def run_case(case, bin_dir, runner, baseline, cwd, timeout):
    name = case["bin"]
    binary = (bin_dir / name).absolute()
    row = {"bin": name, "args": case.get("args", [])}
    if not binary.is_file():
        return dict(row, status="skipped", reason="guest binary missing")
    if not is_x86_64_elf(binary):
        return dict(row, status="skipped", reason="guest binary is not an x86-64 ELF")
    row["binary_sha256"] = digest(binary)
    if baseline is not None:
        expected = baseline.get(name)
        if not expected or expected.get("status") != "passed":
            return dict(row, status="skipped", reason="missing successful native reference")
        if expected.get("binary_sha256") != row["binary_sha256"]:
            return dict(row, status="skipped", reason="guest binary differs from native reference")
    try:
        result = execute(binary, row["args"], case.get("stdin", ""), runner, cwd, timeout)
    except (OSError, subprocess.TimeoutExpired) as exc:
        return dict(row, status="failed", reason=str(exc))
    row["result"] = result
    if result["exit"] != case.get("exit", 0):
        return dict(row, status="failed", reason="unexpected exit status")
    if baseline is not None and result != baseline[name]["result"]:
        return dict(row, status="failed", reason="stdout or stderr differs from native")
    return dict(row, status="passed")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--backend", required=True, choices=["native", "qemu", "prisma"])
    parser.add_argument("--bin-dir", type=Path, default=Path("/usr/bin"))
    parser.add_argument("--baseline", type=Path, help="JSON captured by --backend native")
    parser.add_argument("--runner", help="ELF runner executable (defaults to qemu-x86_64 for QEMU)")
    parser.add_argument("--runner-arg", action="append", default=[], help="One runner argument")
    parser.add_argument("--timeout", type=float, default=10.0, help="Per-case timeout in seconds")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    if args.timeout <= 0:
        parser.error("--timeout must be positive")

    cases = json.loads(CASES.read_text(encoding="utf-8"))
    manifest_sha = digest(CASES)
    baseline = None
    runner = []
    if args.backend == "native":
        if platform.system() != "Linux" or platform.machine() not in {"x86_64", "AMD64"}:
            parser.error("capture native results on an x86_64 Linux host")
        if args.runner or args.runner_arg or args.baseline:
            parser.error("native capture directly executes the guest binaries")
    else:
        if args.baseline is None:
            parser.error("a native --baseline is required")
        if args.baseline.resolve() == args.output.resolve():
            parser.error("the comparison needs a different output file from the native baseline")
        try:
            reference = json.loads(args.baseline.read_text(encoding="utf-8"))
            if (not isinstance(reference, dict) or reference.get("backend") != "native"
                    or reference.get("schema") != SCHEMA
                    or reference.get("corpus_sha256") != manifest_sha
                    or not isinstance(reference.get("cases"), list)):
                raise ValueError("expected a native baseline for this corpus revision")
            baseline = {r["bin"]: r for r in reference["cases"]}
            for record in baseline.values():
                if record["status"] == "passed":
                    actual = record["result"]
                    if (not isinstance(actual["exit"], int)
                            or not isinstance(actual["stdout_b64"], str)
                            or not isinstance(actual["stderr_b64"], str)
                            or not isinstance(record["binary_sha256"], str)):
                        raise ValueError("invalid successful native record")
        except (OSError, TypeError, KeyError, ValueError) as exc:
            parser.error(f"invalid baseline: {exc}")
        runner_path = shutil.which(args.runner) if args.runner else None
        if not args.runner and args.backend == "qemu":
            runner_path = shutil.which("qemu-x86_64") or shutil.which("qemu-x86_64-static")
        if not runner_path:
            parser.error(f"provide an executable --runner for {args.backend}")
        runner = [str(Path(runner_path).absolute()), *args.runner_arg]

    with tempfile.TemporaryDirectory() as workdir:
        rows = [run_case(case, args.bin_dir, runner, baseline, workdir, args.timeout)
                for case in cases]
    counts = Counter(row["status"] for row in rows)
    report = {
        "schema": SCHEMA,
        "corpus_sha256": manifest_sha,
        "backend": args.backend,
        "host": {"system": platform.system(), "machine": platform.machine(),
                 "kernel": platform.release()},
        "bin_dir": str(args.bin_dir.absolute()),
        "runner": runner,
        "runner_sha256": digest(Path(runner[0])) if runner else None,
        "native_baseline_sha256": digest(args.baseline) if args.baseline else None,
        "total": len(cases),
        "passed": counts["passed"],
        "failed": counts["failed"],
        "skipped": counts["skipped"],
        "percent_passed": 100 * counts["passed"] / len(cases),
        "cases": rows,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(f"{args.backend}: {report['passed']}/{report['total']} passed, "
          f"{report['failed']} failed, {report['skipped']} skipped ({report['percent_passed']:.1f}%)")
    if args.backend == "native":
        return int(report["failed"] != 0 or not report["passed"])
    return int(100 * report["passed"] < 85 * report["total"])


if __name__ == "__main__":
    sys.exit(main())
