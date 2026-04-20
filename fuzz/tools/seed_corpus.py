#!/usr/bin/env python3
"""Seed the decoder fuzz corpus with raw-byte inputs for every known opcode.

Run this once after cloning the repo (or when the decoder gains more
opcodes):

    python3 fuzz/tools/seed_corpus.py

Writes files named by opcode mnemonic under fuzz/corpus/decoder/.
Each file is a minimal valid encoding of the instruction — the
fuzzer mutates from there.
"""

from __future__ import annotations

import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
CORPUS = ROOT / "fuzz" / "corpus" / "decoder"

SEEDS: dict[str, bytes] = {
    # No operands / immediates
    "nop":          bytes([0x90]),
    "ret":          bytes([0xC3]),
    "mov_rax_imm":  bytes([0x48, 0xB8, 0x2A, 0, 0, 0, 0, 0, 0, 0]),

    # Register-register, 64-bit
    "mov_rr":       bytes([0x48, 0x89, 0xD8]),     # mov rax, rbx
    "mov_r_rm":     bytes([0x48, 0x8B, 0xC3]),     # mov rax, rbx (via 8B)
    "add_rr":       bytes([0x48, 0x01, 0xD8]),     # add rax, rbx
    "sub_rr":       bytes([0x48, 0x29, 0xD8]),     # sub rax, rbx
    "and_rr":       bytes([0x48, 0x21, 0xD8]),     # and rax, rbx
    "or_rr":        bytes([0x48, 0x09, 0xD8]),     # or  rax, rbx
    "xor_rr":       bytes([0x48, 0x31, 0xD8]),     # xor rax, rbx (common zero-idiom)
    "xor_self":     bytes([0x48, 0x31, 0xC0]),     # xor rax, rax
    "cmp_rr":       bytes([0x48, 0x39, 0xD8]),     # cmp rax, rbx
    "test_rr":      bytes([0x48, 0x85, 0xD8]),     # test rax, rbx
    "inc_rax":      bytes([0x48, 0xFF, 0xC0]),     # inc rax
    "dec_rax":      bytes([0x48, 0xFF, 0xC8]),     # dec rax
    "neg_rax":      bytes([0x48, 0xF7, 0xD8]),     # neg rax
    "not_rax":      bytes([0x48, 0xF7, 0xD0]),     # not rax

    # Control flow
    "jmp_short":    bytes([0xEB, 0x05]),
    "jmp_long":     bytes([0xE9, 0x00, 0x01, 0x00, 0x00]),
    "je":           bytes([0x74, 0x10]),
    "jne":          bytes([0x75, 0x10]),

    # Memory
    "mov_mem_load": bytes([0x48, 0x8B, 0x03]),     # mov rax, [rbx]
    "mov_mem_store": bytes([0x48, 0x89, 0x03]),    # mov [rbx], rax
    "mov_mem_disp8": bytes([0x48, 0x8B, 0x43, 0x08]),
    "mov_mem_disp32": bytes([0x48, 0x8B, 0x83, 0x10, 0x00, 0x00, 0x00]),

    # Intentionally-invalid inputs to train the fuzzer on rejection paths.
    "truncated_rex":     bytes([0x48]),
    "truncated_mov_imm": bytes([0x48, 0xB8, 0x01]),
    "unknown_opcode":    bytes([0x62]),
    "ff_reserved":       bytes([0x48, 0xFF, 0xFF]),  # /7 is reserved on FF
}


def main() -> int:
    CORPUS.mkdir(parents=True, exist_ok=True)
    written = 0
    for name, payload in SEEDS.items():
        path = CORPUS / f"{name}.bin"
        path.write_bytes(payload)
        written += 1
    print(f"seeded {written} corpus files under {CORPUS.relative_to(ROOT)}/")
    return 0


if __name__ == "__main__":
    sys.exit(main())
