#!/usr/bin/env python3
"""Apply the PR #312 ADC/SBB destination-publication ordering fix.

The helper edits only a runner checkout. It is used to validate the source
before the Git data API publishes the exact resulting blob to the product PR.
"""

from __future__ import annotations

import argparse
from pathlib import Path


VALUE_OLD = '''    stmts.push(Stmt::new(
        None,
        Op::StoreCarry(StoreCarry { value: new_cf }),
    ));
    push_adc_sbb_rflags_bits(stmts, is_sbb, lhs, rhs, res, size);
    res
'''

VALUE_NEW = '''    stmts.push(Stmt::new(
        None,
        Op::StoreCarry(StoreCarry { value: new_cf }),
    ));
    // The destination must be published before the expanded PF/AF/ZF/SF/OF
    // graph allocates more SSA refs. The current migration backend maps refs
    // through a bounded register ring, so delaying StoreReg/StoreMem can let a
    // later flag constant overwrite the arithmetic result's physical register.
    res
'''

REG_OLD = '''    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: dst_reg,
            value: res,
            size,
        }),
    ));
}
'''

REG_NEW = '''    stmts.push(Stmt::new(
        None,
        Op::StoreReg(StoreReg {
            reg: dst_reg,
            value: res,
            size,
        }),
    ));
    push_adc_sbb_rflags_bits(stmts, is_sbb, lhs, rhs, res, size);
}
'''

MEM_OLD = '''        stmts.push(Stmt::new(
            None,
            Op::StoreMem(StoreMem {
                addr: addr_ref,
                value: result,
                size,
            }),
        ));
        Ok(1 + used)
'''

MEM_NEW = '''        stmts.push(Stmt::new(
            None,
            Op::StoreMem(StoreMem {
                addr: addr_ref,
                value: result,
                size,
            }),
        ));
        push_adc_sbb_rflags_bits(stmts, is_sbb, dst, src, result, size);
        Ok(1 + used)
'''

IMM_MEM_OLD = '''        if matches!(modrm.reg, 2 | 3) {
            let result = emit_adc_sbb_value(stmts, modrm.reg == 3, mem_val, imm_ref, size);
            stmts.push(Stmt::new(
                None,
                Op::StoreMem(StoreMem {
                    addr: addr_ref,
                    value: result,
                    size,
                }),
            ));
            return Ok(1 + modrm_bytes + imm_bytes);
        }
'''

IMM_MEM_NEW = '''        if matches!(modrm.reg, 2 | 3) {
            let is_sbb = modrm.reg == 3;
            let result = emit_adc_sbb_value(stmts, is_sbb, mem_val, imm_ref, size);
            stmts.push(Stmt::new(
                None,
                Op::StoreMem(StoreMem {
                    addr: addr_ref,
                    value: result,
                    size,
                }),
            ));
            push_adc_sbb_rflags_bits(stmts, is_sbb, mem_val, imm_ref, result, size);
            return Ok(1 + modrm_bytes + imm_bytes);
        }
'''

REG64_ASSERT_OLD = '''            assert!(matches!(
                d.stmts.last().unwrap().op,
                Op::StoreReg(StoreReg {
                    reg: Gpr::Rax,
                    size: OpSize::I64,
                    ..
                })
            ));
'''

REG64_ASSERT_NEW = '''            let destination_index = d
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
                .expect("ADC/SBB must publish RAX");
            let rflags_index = d
                .stmts
                .iter()
                .position(|stmt| matches!(&stmt.op, Op::StoreRflagsFromBits(_)))
                .expect("ADC/SBB must publish RFLAGS");
            assert!(destination_index < rflags_index);
'''

REG_SIZE_ASSERT_OLD = '''            assert!(matches!(
                d.stmts.last().unwrap().op,
                Op::StoreReg(StoreReg {
                    reg: Gpr::Rax,
                    size: actual,
                    ..
                }) if actual == size
            ));
'''

REG_SIZE_ASSERT_NEW = '''            let destination_index = d
                .stmts
                .iter()
                .position(|stmt| {
                    matches!(
                        &stmt.op,
                        Op::StoreReg(StoreReg {
                            reg: Gpr::Rax,
                            size: actual,
                            ..
                        }) if *actual == size
                    )
                })
                .expect("ADC/SBB must publish RAX");
            let rflags_index = d
                .stmts
                .iter()
                .position(|stmt| matches!(&stmt.op, Op::StoreRflagsFromBits(_)))
                .expect("ADC/SBB must publish RFLAGS");
            assert!(destination_index < rflags_index);
'''

MEM_ASSERT_OLD = '''            assert!(matches!(d.stmts.last().unwrap().op, Op::StoreMem(_)));
'''

MEM_ASSERT_NEW = '''            let destination_index = d
                .stmts
                .iter()
                .position(|stmt| matches!(&stmt.op, Op::StoreMem(_)))
                .expect("ADC/SBB must publish memory destination");
            let rflags_index = d
                .stmts
                .iter()
                .position(|stmt| matches!(&stmt.op, Op::StoreRflagsFromBits(_)))
                .expect("ADC/SBB must publish RFLAGS");
            assert!(destination_index < rflags_index);
'''


def replace_exact(text: str, old: str, new: str, name: str, expected: int = 1) -> str:
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
    text = replace_exact(text, VALUE_OLD, VALUE_NEW, "value helper")
    text = replace_exact(text, REG_OLD, REG_NEW, "register destination")
    text = replace_exact(text, MEM_OLD, MEM_NEW, "register-source memory destination")
    text = replace_exact(text, IMM_MEM_OLD, IMM_MEM_NEW, "immediate memory destination")
    text = replace_exact(text, REG64_ASSERT_OLD, REG64_ASSERT_NEW, "I64 register assertion")
    text = replace_exact(
        text,
        REG_SIZE_ASSERT_OLD,
        REG_SIZE_ASSERT_NEW,
        "sized register assertion",
        expected=2,
    )
    text = replace_exact(text, MEM_ASSERT_OLD, MEM_ASSERT_NEW, "memory assertion")
    path.write_text(text, encoding="utf-8")


if __name__ == "__main__":
    main()
