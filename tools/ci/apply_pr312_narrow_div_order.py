#!/usr/bin/env python3
"""Reorder narrow DIV/IDIV temporary construction in a runner checkout.

Build the full dividend before extending and checking the divisor. This keeps
both operands from occupying the same bounded Rust backend register-ring slot,
while preserving #DE semantics because no architectural state is stored before
the divisor and quotient-overflow traps complete.
"""

from __future__ import annotations

import argparse
from pathlib import Path


OLD_PREFIX = '''    let divisor = emit_extend_to_i64(stmts, rhs_ref, size, is_signed);
    emit_divisor_zero_trap(stmts, divisor);
    let dividend = if size == OpSize::I8 {
'''

NEW_PREFIX = '''    let dividend = if size == OpSize::I8 {
'''

OLD_SUFFIX = '''    };

    let quotient = push_binop_ref(
'''

NEW_SUFFIX = '''    };

    // Materialize the divisor only after the dividend is complete. The Rust
    // migration backend currently maps SSA refs through an eight-register ring;
    // extending the divisor earlier let a later shifted-high dividend ref reuse
    // and overwrite its physical register before UDIV/SDIV consumed it.
    // All work above is temporary-only, so divisor-zero and quotient-overflow
    // traps still occur before RAX/RDX are architecturally modified.
    let divisor = emit_extend_to_i64(stmts, rhs_ref, size, is_signed);
    emit_divisor_zero_trap(stmts, divisor);

    let quotient = push_binop_ref(
'''


def replace_once(text: str, old: str, new: str, name: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"expected one {name} block, found {count}")
    return text.replace(old, new)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("checkout", type=Path)
    args = parser.parse_args()

    path = args.checkout / "shell/prisma-decoder/src/decode.rs"
    text = path.read_text(encoding="utf-8")
    text = replace_once(text, OLD_PREFIX, NEW_PREFIX, "early divisor")
    text = replace_once(text, OLD_SUFFIX, NEW_SUFFIX, "post-dividend insertion")
    path.write_text(text, encoding="utf-8")


if __name__ == "__main__":
    main()
