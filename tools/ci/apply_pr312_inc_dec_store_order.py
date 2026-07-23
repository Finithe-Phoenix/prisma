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
        1,
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
        1,
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
        1,
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
        1,
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
        1,
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
        1,
    ),
    (
        '''        assert!(matches!(
            inc.stmts.last().unwrap().op,
            Op::StoreMem(StoreMem {
                size: OpSize::I8,
                ..
            })
        ));
''',
        '''        let inc_destination_index = inc
            .stmts
            .iter()
            .position(|stmt| {
                matches!(
                    &stmt.op,
                    Op::StoreMem(StoreMem {
                        size: OpSize::I8,
                        ..
                    })
                )
            })
            .expect("INC must publish its memory destination");
        let inc_rflags_index = inc
            .stmts
            .iter()
            .position(|stmt| matches!(&stmt.op, Op::StoreRflagsFromNzcv(_)))
            .expect("INC must publish RFLAGS");
        assert!(inc_destination_index < inc_rflags_index);
''',
        "Group 4 INC memory assertion",
        1,
    ),
    (
        '''        assert!(matches!(
            dec.stmts.last().unwrap().op,
            Op::StoreMem(StoreMem {
                size: OpSize::I8,
                ..
            })
        ));
''',
        '''        let dec_destination_index = dec
            .stmts
            .iter()
            .position(|stmt| {
                matches!(
                    &stmt.op,
                    Op::StoreMem(StoreMem {
                        size: OpSize::I8,
                        ..
                    })
                )
            })
            .expect("DEC must publish its memory destination");
        let dec_rflags_index = dec
            .stmts
            .iter()
            .position(|stmt| matches!(&stmt.op, Op::StoreRflagsFromNzcv(_)))
            .expect("DEC must publish RFLAGS");
        assert!(dec_destination_index < dec_rflags_index);
''',
        "Group 4 DEC memory assertion",
        1,
    ),
    (
        '''        assert_eq!(
            inc.stmts.last().unwrap(),
            &Stmt::new(
                None,
                Op::StoreReg(StoreReg {
                    reg: Gpr::Rax,
                    value: 2,
                    size: OpSize::I64,
                }),
            )
        );
''',
        '''        let inc_destination_index = inc
            .stmts
            .iter()
            .position(|stmt| {
                matches!(
                    &stmt.op,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        value: 2,
                        size: OpSize::I64,
                    })
                )
            })
            .expect("INC must publish RAX");
        let inc_rflags_index = inc
            .stmts
            .iter()
            .position(|stmt| matches!(&stmt.op, Op::StoreRflagsFromNzcv(_)))
            .expect("INC must publish RFLAGS");
        assert!(inc_destination_index < inc_rflags_index);
''',
        "Group 5 INC register assertion",
        1,
    ),
    (
        '''        assert!(matches!(
            dec.stmts.last().unwrap().op,
            Op::StoreReg(StoreReg {
                reg: Gpr::Rax,
                size: OpSize::I64,
                ..
            })
        ));
''',
        '''        let dec_destination_index = dec
            .stmts
            .iter()
            .position(|stmt| {
                matches!(
                    &stmt.op,
                    Op::StoreReg(StoreReg {
                        reg: Gpr::Rax,
                        size: OpSize::I64,
                        ..
                    })
                )
            })
            .expect("DEC must publish RAX");
        let dec_rflags_index = dec
            .stmts
            .iter()
            .position(|stmt| matches!(&stmt.op, Op::StoreRflagsFromNzcv(_)))
            .expect("DEC must publish RFLAGS");
        assert!(dec_destination_index < dec_rflags_index);
''',
        "Group 5 DEC register assertion",
        1,
    ),
    (
        '''        assert!(matches!(inc.stmts.last().unwrap().op, Op::StoreMem(_)));
''',
        '''        let inc_destination_index = inc
            .stmts
            .iter()
            .position(|stmt| matches!(&stmt.op, Op::StoreMem(_)))
            .expect("INC must publish its memory destination");
        let inc_rflags_index = inc
            .stmts
            .iter()
            .position(|stmt| matches!(&stmt.op, Op::StoreRflagsFromNzcv(_)))
            .expect("INC must publish RFLAGS");
        assert!(inc_destination_index < inc_rflags_index);
''',
        "Group 5 memory INC assertions",
        2,
    ),
    (
        '''        assert!(matches!(dec.stmts.last().unwrap().op, Op::StoreMem(_)));
''',
        '''        let dec_destination_index = dec
            .stmts
            .iter()
            .position(|stmt| matches!(&stmt.op, Op::StoreMem(_)))
            .expect("DEC must publish its memory destination");
        let dec_rflags_index = dec
            .stmts
            .iter()
            .position(|stmt| matches!(&stmt.op, Op::StoreRflagsFromNzcv(_)))
            .expect("DEC must publish RFLAGS");
        assert!(dec_destination_index < dec_rflags_index);
''',
        "Group 5 memory DEC assertions",
        2,
    ),
]


def replace_exact(text: str, old: str, new: str, name: str, expected: int) -> str:
    count = text.count(old)
    if count != expected:
        raise SystemExit(f"expected {expected} {name} block(s), found {count}")
    return text.replace(old, new)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("checkout", type=Path)
    args = parser.parse_args()

    path = args.checkout / "shell/prisma-decoder/src/decode.rs"
    text = path.read_text(encoding="utf-8")
    for old, new, name, expected in REPLACEMENTS:
        text = replace_exact(text, old, new, name, expected)
    path.write_text(text, encoding="utf-8")


if __name__ == "__main__":
    main()
