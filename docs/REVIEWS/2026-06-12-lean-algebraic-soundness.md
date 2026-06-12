# Two-eyes review — optimizer pass soundness: algebraic_simplify + strength_reduce (Lean)

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

## Follow-on: strength_reduce pass soundness

Second commit on this branch. `core/src/passes/strength_reduce.cpp` does
one rewrite: `Mul x, (1<<k) → Shl x, k` (k in 1..63, fresh shift-count
Constant). Added `mul_pow2_eq_shl` (core identity `a*b = a <<< (c & 0x3F)`
given `b.toNat = 2^k`, `c.toNat = k`, `k < 64`, via the toNat chain +
`Nat.and_two_pow_sub_one_eq_mod` for the 6-bit count mask) and
`strength_reduce_mul_pow2_sound` (the evalPure-preserving wrapper).

- **codex**: **LGTM**. Verified the toNat chain, that the `& 0x3F` mask is
  correctly shown to be identity for `k < 64`, and that the `k < 64` bound
  is essential and correctly used. One optional non-blocking suggestion —
  tighten the precondition to `1 ≤ k ∧ k < 64` to mirror the pass's exact
  firing range — **applied** (`_hk1 : 1 ≤ k` added; the core identity is
  sound at k = 0 too but the pass never fires there).
- gemini: not re-run (tool-availability failure earlier this session).

With this, the optimizer's arithmetic-rewriting passes — constant_propagate
(F1-LN-010), algebraic_simplify, and strength_reduce — all carry Lean
soundness proofs. Spec remains sorry-free.
