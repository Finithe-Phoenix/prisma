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


REG_TEMPLATE = '''        emit_alu_flags_preserve_carry(stmts, {kind}, {value}, one, size);
        stmts.push(Stmt::new(
            None,
            Op::StoreReg(StoreReg {{
                reg: {reg},
                value: {result},
                size,
            }}),
        ));
'''

REG_REPLACEMENT = '''        stmts.push(Stmt::new(
            None,
            Op::StoreReg(StoreReg {{
                reg: {reg},
                value: {result},
                size,
            }}),
        ));
        emit_alu_flags_preserve_carry(stmts, {kind}, {value}, one, size);
'''

MEM_TEMPLATE = '''        emit_alu_flags_preserve_carry(stmts, {kind}, {value}, one, size);
        stmts.push(Stmt::new(
            None,
            Op::StoreMem(StoreMem {{
                addr: {addr},
                value: {result},
                size,
            }}),
        ));
'''

MEM_REPLACEMENT = '''        stmts.push(Stmt::new(
            None,
            Op::StoreMem(StoreMem {{
                addr: {addr},
                value: {result},
                size,
            }}),
        ));
        emit_alu_flags_preserve_carry(stmts, {kind}, {value}, one, size);
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

    replacements = [
        (
            REG_TEMPLATE.format(kind="kind", value="value", reg="reg", result="result"),
            REG_REPLACEMENT.format(kind="kind", value="value", reg="reg", result="result"),
            "Group 4 register",
        ),
        (
            MEM_TEMPLATE.format(kind="kind", value="value", addr="addr", result="result"),
            MEM_REPLACEMENT.format(kind="kind", value="value", addr="addr", result="result"),
            "Group 4 memory",
        ),
        (
            REG_TEMPLATE.format(
                kind="BinOpKind::Add",
                value="value_ref",
                reg="dst_reg",
                result="result_ref",
            ),
            REG_REPLACEMENT.format(
                kind="BinOpKind::Add",
                value="value_ref",
                reg="dst_reg",
                result="result_ref",
            ),
            "Group 5 INC register",
        ),
        (
            MEM_TEMPLATE.format(
                kind="BinOpKind::Add",
                value="value_ref",
                addr="addr_ref",
                result="result_ref",
            ),
            MEM_REPLACEMENT.format(
                kind="BinOpKind::Add",
                value="value_ref",
                addr="addr_ref",
                result="result_ref",
            ),
            "Group 5 INC memory",
        ),
        (
            REG_TEMPLATE.format(
                kind="BinOpKind::Sub",
                value="value_ref",
                reg="dst_reg",
                result="result_ref",
            ),
            REG_REPLACEMENT.format(
                kind="BinOpKind::Sub",
                value="value_ref",
                reg="dst_reg",
                result="result_ref",
            ),
            "Group 5 DEC register",
        ),
        (
            MEM_TEMPLATE.format(
                kind="BinOpKind::Sub",
                value="value_ref",
                addr="addr_ref",
                result="result_ref",
            ),
            MEM_REPLACEMENT.format(
                kind="BinOpKind::Sub",
                value="value_ref",
                addr="addr_ref",
                result="result_ref",
            ),
            "Group 5 DEC memory",
        ),
    ]

    for old, new, name in replacements:
        text = replace_once(text, old, new, name)

    path.write_text(text, encoding="utf-8")


if __name__ == "__main__":
    main()
