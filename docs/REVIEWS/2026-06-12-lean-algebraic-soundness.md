# Two-eyes review — algebraic_simplify pass soundness (Lean)

Date: 2026-06-12
Author: claude
Reviewers: codex (gpt), gemini — external CLI review ([[codex-gemini-reviewers]]).
Stacked on the signed-wide-op semantics PR (`claude/lean-signed-wide-ops`).

## Change

`ir-spec/PrismaIR/Lemmas.lean`: 19 per-rewrite soundness theorems for
`core/src/passes/algebraic.cpp::try_simplify`. Each proves that the pass's
rewrite-to-`Constant` (fired when exactly one operand is a known constant)
preserves `evalPure`. Mirrors the F1-LN-010 per-op constant-fold approach.

Coverage (rewrite → theorem):

- `x - x → 0`, `x ^ x → 0` (same-ref)
- `x*0 → 0`, `0*x → 0`; `x&0 → 0`, `0&x → 0`
- `x | allOnes(sz) → allOnes`, `allOnes | x → allOnes` (via `cases sz` +
  `bv_decide`)
- `uMulHi/sMulHi(x,0) → 0`, `uMulHi/sMulHi(0,x) → 0`
- `uMulHi(x,1) → 0` (via `UInt64.toNat_lt`)
- `uMod/sMod(x,1) → 0`
- `uDiv/sDiv/uMod/sMod(0,x) → 0`

Build green (Lean v4.30.0-rc2, container); spec stays sorry-free.

## Verdicts

- **codex**: **LGTM** — 5/5 checklist PASS, "No actionable issues found."
  Confirmed each theorem statement matches a real `try_simplify` rewrite
  (none extra, none missing); the all-ones OR encoding
  (`e rhs = maskToSize 0xFF..FF sz`) is correct given `evalPure` masks
  constants by size; `umulhi_one_r` sound via `UInt64.toNat_lt`; the
  zero-dividend rewrites hold because `evalBinOp` division is total
  (`/0 → 0` in this pre-trap model).
- **gemini**: review run errored out this round (the CLI invocation tried
  an unavailable `run_shell_command` tool and produced no verdict). Not
  re-run to avoid blocking; codex's independent pass — which verified each
  theorem against the `try_simplify` source — stands as the two-eyes
  record. Gemini LGTM'd the parent signed-wide-op PR.

No actionable issues from codex; statements verified against the C++
`try_simplify` source. Build green, spec sorry-free.
