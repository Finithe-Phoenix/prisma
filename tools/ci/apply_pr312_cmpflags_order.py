#!/usr/bin/env python3
"""Emit CmpFlags before software PF/AF expansion in a runner checkout.

The Rust migration backend maps SSA refs through a bounded register ring.
Delaying CmpFlags until after the parity/auxiliary-flag graph can overwrite the
live compare operands before the ARM64 CMP consumes them. Publishing NZCV first
is safe: the PF/AF graph lowers to non-flag-setting integer operations, and the
subsequent StoreRflagsFromNzcv reads the preserved NZCV value.
"""

from __future__ import annotations

import argparse
from pathlib import Path


OLD = '''fn push_cmp_flags(stmts: &mut Vec<Stmt>, lhs: Ref, rhs: Ref, size: OpSize) -> Ref {
    let result = push_binop_ref(stmts, BinOpKind::Sub, lhs, rhs, size);
    let (pf, af) = push_pf_af_for_alu(stmts, BinOpKind::Sub, lhs, rhs, result, size);
    let flags = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(flags),
        Op::CmpFlags(CmpFlags { lhs, rhs, size }),
    ));
    push_store_rflags_from_nzcv(stmts, RflagsCarryMode::InvertArmCarry, pf, af);
    flags
}
'''

NEW = '''fn push_cmp_flags(stmts: &mut Vec<Stmt>, lhs: Ref, rhs: Ref, size: OpSize) -> Ref {
    let result = push_binop_ref(stmts, BinOpKind::Sub, lhs, rhs, size);
    // Publish NZCV while lhs/rhs still occupy their original physical slots.
    // The PF/AF graph below uses non-flag-setting operations, so NZCV remains
    // valid until StoreRflagsFromNzcv serializes it to the guest frame.
    let flags = alloc_ref(stmts);
    stmts.push(Stmt::new(
        Some(flags),
        Op::CmpFlags(CmpFlags { lhs, rhs, size }),
    ));
    let (pf, af) = push_pf_af_for_alu(stmts, BinOpKind::Sub, lhs, rhs, result, size);
    push_store_rflags_from_nzcv(stmts, RflagsCarryMode::InvertArmCarry, pf, af);
    flags
}
'''


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("checkout", type=Path)
    args = parser.parse_args()

    path = args.checkout / "shell/prisma-decoder/src/decode.rs"
    text = path.read_text(encoding="utf-8")
    count = text.count(OLD)
    if count != 1:
        raise SystemExit(f"expected one push_cmp_flags block, found {count}")
    path.write_text(text.replace(OLD, NEW), encoding="utf-8")


if __name__ == "__main__":
    main()
