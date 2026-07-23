#!/usr/bin/env python3
"""Apply the minimal unsigned WideDiv register-pressure fix to a checkout.

This helper is used only by the CI-stabilization PR to produce and validate a
source artifact. It does not commit or push anything.
"""

from __future__ import annotations

import argparse
from pathlib import Path


OLD = '''          WideDivWorkRegs work{};
          if (!allocate_temporary(work.rem) || !allocate_temporary(work.low) ||
              !allocate_temporary(work.divisor) || !allocate_temporary(work.mask) ||
              !allocate_temporary(work.bit) || !allocate_temporary(work.overflow)) {
            return {false, LowerError::OutOfScratchRegs, "WideDiv temporaries"};
          }

          emit_divisor_zero_sigfpe_guard(emitter_, divisor, options_.emit_ret_on_terminator);
          emitter_.mov_reg_reg(work.rem, high);
          emitter_.mov_reg_reg(work.low, low);
          emitter_.mov_reg_reg(work.divisor, divisor);
          if (op.is_signed) {
'''

NEW = '''          WideDivWorkRegs work{};
          if (!allocate_temporary(work.rem) || !allocate_temporary(work.low) ||
              !allocate_temporary(work.mask) || !allocate_temporary(work.bit) ||
              !allocate_temporary(work.overflow)) {
            return {false, LowerError::OutOfScratchRegs, "WideDiv temporaries"};
          }
          if (op.is_signed) {
            if (!allocate_temporary(work.divisor)) {
              return {false, LowerError::OutOfScratchRegs,
                      "WideDiv signed divisor temporary"};
            }
          } else {
            // Unsigned division never mutates the divisor. Reuse its live
            // SSA register instead of consuming a sixth temporary.
            work.divisor = divisor;
          }

          emit_divisor_zero_sigfpe_guard(emitter_, divisor, options_.emit_ret_on_terminator);
          emitter_.mov_reg_reg(work.rem, high);
          emitter_.mov_reg_reg(work.low, low);
          if (op.is_signed) {
            emitter_.mov_reg_reg(work.divisor, divisor);
'''


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("checkout", type=Path)
    args = parser.parse_args()

    path = args.checkout / "core/src/backend/lowering.cpp"
    text = path.read_text(encoding="utf-8")
    count = text.count(OLD)
    if count != 1:
        raise SystemExit(f"expected one WideDiv allocation block, found {count}")
    path.write_text(text.replace(OLD, NEW), encoding="utf-8")


if __name__ == "__main__":
    main()
