# Two-eyes review — Lean signed wide-op semantics (F1-LN-014/015/016)

Date: 2026-06-12
Author: claude
Reviewers: codex (gpt), gemini — external CLI review per the two-eyes
protocol ([[codex-gemini-reviewers]]).

## Change

`ir-spec/PrismaIR/Semantics.lean`: replace the three `sorry` placeholders
for the signed wide-form ops (`sMulHi`, `sDiv`, `sMod`) in `evalBinOp`
with concrete total definitions, plus two helpers (`toSignedInt`,
`ofSignedInt`). The semantics mirror the authoritative ARM64-matching
fold in `core/src/passes/const_prop.cpp` exactly:

- `sMulHi` = upper 64 bits of the two's-complement 128-bit signed product.
- `sDiv` = `0` on divide-by-zero, `INT_MIN` on `INT_MIN / -1` (wraps, no
  `#DE` at the pure-eval level), else truncation toward zero (`Int.tdiv`).
- `sMod` = dividend on divide-by-zero, `0` on `INT_MIN / -1`, else
  remainder with the sign of the dividend (`Int.tmod`).

`ir-spec/PrismaIR/Lemmas.lean`: 11 executable conformance `example`s
(`by decide`) pinning the corner cases. `ir-spec/.sorry-budget`: 3 → 0.

Verified: `lake build` green (Lean v4.30.0-rc2, container), `.sorry-budget`
gate sees 0 sorries.

## Verdicts

Both reviewers returned **LGTM** with all six checklist items PASS and no
issues (BLOCKER/MAJOR/MINOR: none).

- **codex**: confirmed `toSignedInt` boundary at `2^63`; `ofSignedInt`
  Euclidean-mod wrap correct for negatives; `sMulHi` upper-word extraction
  correct for both signs (checked `(-1)*1 → 0xFFFF…FF`); `Int.tdiv`/`tmod`
  are the trunc-toward-zero match for C `/`/`%`; 11 examples consistent;
  budget 3→0 consistent. Independently ran `rg sorry ir-spec/` — only the
  comment reference remains.
- **gemini**: same six PASS, explicitly validated the negative-remainder
  cases (`-7 % 2 = -1`), the `INT_MIN / -1` corner, and that `by decide`
  confirms computational correctness in-kernel.

No false positives this round; both verified against the C++ reference
embedded in the review prompt.
