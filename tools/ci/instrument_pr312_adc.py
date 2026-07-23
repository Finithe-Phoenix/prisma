#!/usr/bin/env python3
"""Instrument PR #312 ADC/SBB execution tests in a runner checkout only."""

from __future__ import annotations

import argparse
from pathlib import Path


OLD = '''fn translate(addr: u64, program: &[u8]) -> Vec<u8> {
    let mut t = Translator::new();
    let block = t
        .translate_fused_block(addr, program, 64)
        .expect("fused block translation");
'''

NEW = '''fn translate(addr: u64, program: &[u8]) -> Vec<u8> {
    let mut t = Translator::new();
    let opt = t
        .optimize_fused_block(addr, program, 64)
        .expect("fused block optimization");
    eprintln!("PROGRAM={program:02x?}\\nOPTIMIZED_IR={:#?}", opt.func);
    let block = t
        .translate_fused_block(addr, program, 64)
        .expect("fused block translation");
    eprintln!("ARM64_CODE={:02x?}", block.code);
'''


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("checkout", type=Path)
    args = parser.parse_args()

    path = args.checkout / "shell/prisma-runtime/tests/exec_adc.rs"
    text = path.read_text(encoding="utf-8")
    count = text.count(OLD)
    if count != 1:
        raise SystemExit(f"expected one translate helper, found {count}")
    path.write_text(text.replace(OLD, NEW), encoding="utf-8")


if __name__ == "__main__":
    main()
