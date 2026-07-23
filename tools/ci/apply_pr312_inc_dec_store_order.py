#!/usr/bin/env python3
"""Publish INC/DEC destinations before expanded flag temporaries.

The helper edits only a runner checkout. INC/DEC cannot fault after operand
access and arithmetic, so publishing the architectural destination before the
PF/AF expansion is safe and prevents the bounded Rust backend register ring
from overwriting the result ref before StoreReg/StoreMem.
"""

from __future__ import annotations

import argparse
from pathlib import Path


REPLACEMENTS = [
    (
        '''        emit_alu_flags_preserve_carry(stmts, kind, value, one, size);
        stmts.push(Stmt::new(
            None,
            Op::StoreReg(StoreReg {
                reg,
                value: result,
                size,
            }),
        ));
''',
        '''        stmts.push(Stmt::new(
            None,
            Op::StoreReg(StoreReg {
                reg,
                value: result,
                size,
            }),
        ));
        emit_alu_flags_preserve_carry(stmts, kind, value, one, size);
''',
        "Group 4 register",
    ),
    (
        '''        emit_alu_flags_preserve_carry(stmts, kind, value, one, size);
        stmts.push(Stmt::new(
            None,
            Op::StoreMem(StoreMem {
                addr,
                value: result,
                size,
            }),
        ));
''',
        '''        stmts.push(Stmt::new(
            None,
            Op::StoreMem(StoreMem {
                addr,
                value: result,
                size,
            }),
        ));
        emit_alu_flags_preserve_carry(stmts, kind, value, one, size);
''',
        "Group 4 memory",
    ),
    (
        '''                emit_alu_flags_preserve_carry(stmts, BinOpKind::Add, value_ref, one, size);
                stmts.push(Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: dst_reg,
                        value: result_ref,
                        size,
                    }),
                ));
''',
        '''                stmts.push(Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: dst_reg,
                        value: result_ref,
                        size,
                    }),
                ));
                emit_alu_flags_preserve_carry(stmts, BinOpKind::Add, value_ref, one, size);
''',
        "Group 5 INC register",
    ),
    (
        '''                emit_alu_flags_preserve_carry(stmts, BinOpKind::Add, value_ref, one, size);
                stmts.push(Stmt::new(
                    None,
                    Op::StoreMem(StoreMem {
                        addr: addr_ref,
                        value: result_ref,
                        size,
                    }),
                ));
''',
        '''                stmts.push(Stmt::new(
                    None,
                    Op::StoreMem(StoreMem {
                        addr: addr_ref,
                        value: result_ref,
                        size,
                    }),
                ));
                emit_alu_flags_preserve_carry(stmts, BinOpKind::Add, value_ref, one, size);
''',
        "Group 5 INC memory",
    ),
    (
        '''                emit_alu_flags_preserve_carry(stmts, BinOpKind::Sub, value_ref, one, size);
                stmts.push(Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: dst_reg,
                        value: result_ref,
                        size,
                    }),
                ));
''',
        '''                stmts.push(Stmt::new(
                    None,
                    Op::StoreReg(StoreReg {
                        reg: dst_reg,
                        value: result_ref,
                        size,
                    }),
                ));
                emit_alu_flags_preserve_carry(stmts, BinOpKind::Sub, value_ref, one, size);
''',
        "Group 5 DEC register",
    ),
    (
        '''                emit_alu_flags_preserve_carry(stmts, BinOpKind::Sub, value_ref, one, size);
                stmts.push(Stmt::new(
                    None,
                    Op::StoreMem(StoreMem {
                        addr: addr_ref,
                        value: result_ref,
                        size,
                    }),
                ));
''',
        '''                stmts.push(Stmt::new(
                    None,
                    Op::StoreMem(StoreMem {
                        addr: addr_ref,
                        value: result_ref,
                        size,
                    }),
                ));
                emit_alu_flags_preserve_carry(stmts, BinOpKind::Sub, value_ref, one, size);
''',
        "Group 5 DEC memory",
    ),
]


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
    for old, new, name in REPLACEMENTS:
        text = replace_once(text, old, new, name)
    path.write_text(text, encoding="utf-8")


if __name__ == "__main__":
    main()
