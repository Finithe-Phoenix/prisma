#!/usr/bin/env python3
"""F1-TC-003 — QEMU user-mode differential harness for the Prisma decoder.

For each x86_64 instruction byte sequence in the corpus, this harness:

  1. Builds a tiny ELF/Mach-O wrapper that loads the bytes and exits
     via SYS_exit_group with rax holding the architectural value to
     observe.
  2. Runs it through `qemu-x86_64` (user-mode), capturing exit code
     and the requested register snapshot.
  3. (Future) Compares against Prisma's translator output.

Today only step 2 runs end-to-end; step 3 hooks in once the
Translator has a "single-step run" API exposed to Python (planned
alongside F1-AC-003 baseline numbers).

The harness is intentionally a `tools/` script rather than a CMake
test because:
  * QEMU user-mode is not available on macOS via Homebrew (only
    qemu-system-*). The CI Linux runner has it; local macOS dev
    can install via Docker linux/amd64 once Docker Desktop's
    Rosetta layer is fixed for non-trivial binaries.
  * Generating ELFs is more naturally Python than CMake.

Skips cleanly when `qemu-x86_64` is not on PATH.
"""

from __future__ import annotations

import argparse
import os
import shutil
import struct
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import NamedTuple


def find_qemu() -> str | None:
    return shutil.which("qemu-x86_64") or shutil.which("qemu-x86_64-static")


# Minimal Linux x86_64 ELF that sets rax=42 and calls sys_exit_group.
# Encoded by hand — every byte annotated. Used as the trivial "qemu
# is alive and reports exit code" sanity check.
_PROBE_BYTES = bytes(
    [
        # mov edi, 0x2A           ; bf 2a 00 00 00     (exit code = 42)
        0xBF, 0x2A, 0x00, 0x00, 0x00,
        # mov eax, 0xE7           ; b8 e7 00 00 00     (sys_exit_group on x86_64 Linux = 231)
        0xB8, 0xE7, 0x00, 0x00, 0x00,
        # syscall                 ; 0f 05
        0x0F, 0x05,
    ]
)


def build_minimal_elf(code: bytes, out_path: Path) -> None:
    """Build the smallest possible static ELF that runs `code` and dies.

    Layout:
      Ehdr  (64 bytes)
      Phdr  (56 bytes)  loadable at 0x400000, R+X
      code  (right after the headers)
    """
    base = 0x400000
    entry = base + 0x78
    code_offset = 0x78  # ehdr (64) + phdr (56) = 120 = 0x78

    # ELF header for x86_64 little-endian executable.
    e_ident = bytes([0x7F, ord("E"), ord("L"), ord("F"),
                     2, 1, 1, 0,            # EI_CLASS=64, EI_DATA=LE, EI_VERSION=1, EI_OSABI=0
                     0, 0, 0, 0, 0, 0, 0])  # padding
    ehdr = (
        e_ident
        + struct.pack("<H", 2)               # e_type = ET_EXEC
        + struct.pack("<H", 0x3E)            # e_machine = EM_X86_64
        + struct.pack("<I", 1)               # e_version
        + struct.pack("<Q", entry)           # e_entry
        + struct.pack("<Q", 64)              # e_phoff
        + struct.pack("<Q", 0)               # e_shoff
        + struct.pack("<I", 0)               # e_flags
        + struct.pack("<H", 64)              # e_ehsize
        + struct.pack("<H", 56)              # e_phentsize
        + struct.pack("<H", 1)               # e_phnum
        + struct.pack("<H", 0)               # e_shentsize
        + struct.pack("<H", 0)               # e_shnum
        + struct.pack("<H", 0)               # e_shstrndx
    )
    assert len(ehdr) == 64, len(ehdr)

    file_size = code_offset + len(code)
    phdr = (
        struct.pack("<I", 1)                 # p_type = PT_LOAD
        + struct.pack("<I", 5)               # p_flags = R+X
        + struct.pack("<Q", 0)               # p_offset
        + struct.pack("<Q", base)            # p_vaddr
        + struct.pack("<Q", base)            # p_paddr
        + struct.pack("<Q", file_size)       # p_filesz
        + struct.pack("<Q", file_size)       # p_memsz
        + struct.pack("<Q", 0x1000)          # p_align
    )
    assert len(phdr) == 56, len(phdr)

    out_path.write_bytes(ehdr + phdr + code)
    out_path.chmod(0o755)


class RunResult(NamedTuple):
    exit_code: int
    signal: int  # 0 if exited normally
    stdout: bytes
    stderr: bytes


def run_with_qemu(qemu: str, elf: Path, timeout_s: float = 10.0) -> RunResult:
    p = subprocess.run(
        [qemu, str(elf)],
        capture_output=True,
        timeout=timeout_s,
    )
    sig = 0 if p.returncode >= 0 else -p.returncode
    return RunResult(p.returncode, sig, p.stdout, p.stderr)


def cmd_probe(qemu: str) -> int:
    with tempfile.TemporaryDirectory() as td:
        elf = Path(td) / "probe"
        build_minimal_elf(_PROBE_BYTES, elf)
        r = run_with_qemu(qemu, elf)
        if r.exit_code != 42:
            print(f"PROBE FAILED: expected exit 42, got {r.exit_code}", file=sys.stderr)
            print(f"  stderr: {r.stderr.decode(errors='replace')}", file=sys.stderr)
            return 1
        print("probe: ok (qemu-x86_64 reports exit code 42 from a hand-rolled ELF)")
        return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--probe", action="store_true",
                        help="Run the smallest possible smoke test: an ELF that "
                             "exits 42 via syscall. Use this to verify your QEMU "
                             "user-mode setup.")
    args = parser.parse_args()

    qemu = find_qemu()
    if qemu is None:
        print("SKIP: qemu-x86_64 not on PATH. Install via:")
        print("  Linux:  apt install qemu-user / dnf install qemu-user")
        print("  macOS:  Docker Desktop linux/amd64 image with qemu-user-static,")
        print("          or build qemu from source with --enable-linux-user.")
        return 0  # exit 0 so this is a soft-skip in CI

    if args.probe:
        return cmd_probe(qemu)

    # Default: probe + future differential. Today, just probe.
    return cmd_probe(qemu)


if __name__ == "__main__":
    raise SystemExit(main())
