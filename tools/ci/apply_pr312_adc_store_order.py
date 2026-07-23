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
    text = replace_once(text, VALUE_OLD, VALUE_NEW, "value helper")
    text = replace_once(text, REG_OLD, REG_NEW, "register destination")
    text = replace_once(text, MEM_OLD, MEM_NEW, "memory destination")
    path.write_text(text, encoding="utf-8")


if __name__ == "__main__":
    main()
