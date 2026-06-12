/-
  PrismaIR.Lemmas — first few properties of the IR semantics.

  This is the beginning of the Pillar 2 proof trail. Each lemma is either
  a sanity check (the definition behaves as expected) or a step toward a
  bigger theorem (e.g., correctness of a pass). As more opcodes and passes
  arrive, more lemmas join this file.
-/

import PrismaIR.Syntax
import PrismaIR.Semantics
import PrismaIR.MachineState
import Std.Tactic.BVDecide

namespace PrismaIR

/-!
## Determinism of `evalPure`.

`evalPure` is defined as a total function (`Env → Op → Option UInt64`),
so determinism is immediate: given the same environment and the same op,
the result is unique. We state it explicitly to make the property visible
in the spec and to serve as a proof skeleton for later pass soundness
lemmas, which have the same shape but are non-trivial.
-/

theorem evalPure_deterministic
    (e : Env) (op : Op) {v₁ v₂ : UInt64}
    (h₁ : evalPure e op = some v₁)
    (h₂ : evalPure e op = some v₂) :
    v₁ = v₂ := by
  rw [h₁] at h₂
  injection h₂

/-!
## Constant evaluation collapses to a masked value.

For every `constant` op that evaluates, the produced value is exactly
`maskToSize v sz`. This is a simple structural fact that downstream
lowering passes rely on when they decide a constant fits a given operand
size without further masking.
-/

theorem evalPure_constant
    (e : Env) (v : UInt64) (sz : OpSize) (r : UInt64)
    (h : evalPure e (.constant v sz) = some r) :
    r = maskToSize v sz := by
  -- `evalPure e (.constant v sz)` reduces to `some (maskToSize v sz)`.
  -- After `simp`, `h` has the form `maskToSize v sz = r`; flip it.
  simp [evalPure] at h
  exact h.symm

/-!
## Binop evaluation reveals the operation applied to the masked operands.

Similar shape. This lemma is what a later pass uses when it wants to
constant-fold a `BinOp` whose two inputs are known to be constants.
-/

theorem evalPure_binop
    (e : Env) (op : BinOp) (lhs rhs : Ref) (sz : OpSize) (r : UInt64)
    (h : evalPure e (.binop op lhs rhs sz) = some r) :
    r = maskToSize (evalBinOp op (e lhs) (e rhs)) sz := by
  simp [evalPure] at h
  exact h.symm

/-!
## Sanity: the `exampleProgram` reduces as expected.

Concrete check that the semantics compute the arithmetic we expect on a
canonical example. This mirrors the C++ test
`core/tests/test_ir.cpp::"Lean exampleProgram mirrored in C++"`, keeping
the two sides of the spec<->impl boundary honest.
-/

example :
    let e₀ : Env := fun _ => 0
    let e₁ := e₀.extend 0 10
    let e₂ := e₁.extend 1 32
    evalPure e₂ (.binop .add 0 1 .i64) = some 42 := by
  decide

/-!
## Masking idempotence.

`maskToSize (maskToSize v sz) sz = maskToSize v sz` — applying the
size mask twice is the same as once. The proof reduces to four cases
on `sz`; in each case the body is `(v &&& C) &&& C` with `C` a
concrete constant. `bv_decide` is complete on closed bit-vector
goals; with the free `v` we instead lift to `BitVec 64` via the
`UInt64.toBitVec` coercion and apply `BitVec.and_assoc` +
`BitVec.and_self`.
-/

theorem maskToSize_idem (v : UInt64) (sz : OpSize) :
    maskToSize (maskToSize v sz) sz = maskToSize v sz := by
  cases sz <;> simp [maskToSize] <;> first | rfl | bv_decide

/-!
## Local soundness: constant folding of `binop` preserves `evalPure`.

This is the building block under F1-LN-010 (full
`constant_propagate` soundness). When both operands of a `binop`
are known constants in the environment, replacing the `binop`
with a `constant` carrying the folded value computes the same
`evalPure` answer.

The bigger proof for F1-LN-010 quantifies over a `pass` function
acting on a whole `List Stmt` and an `exec → Trace` interpretation.
Both still TODO — but discharging this local lemma pins the
arithmetic equivalence the global proof relies on, so it cannot
silently drift if the C++ pass is changed.
-/

theorem constant_fold_binop_sound
    (e : Env) (op : BinOp) (lhs rhs : Ref) (sz : OpSize)
    (a b folded : UInt64)
    (ha : e lhs = a) (hb : e rhs = b)
    (hf : folded = maskToSize (evalBinOp op a b) sz) :
    evalPure e (.binop op lhs rhs sz)
      = evalPure e (.constant folded sz) := by
  -- LHS reduces to `some (maskToSize (evalBinOp op a b) sz)`.
  -- RHS reduces to `some (maskToSize folded sz)`.
  -- Apply `maskToSize_idem` to collapse the double-mask on the RHS.
  simp [evalPure, ha, hb, hf, maskToSize_idem]

/-!
## Local soundness: `extend` of a known constant matches a
`constant`-direct evaluation.

Building block for the constant-fold case in F1-PS-010 (the C++
pass `constant_propagate` already folds Extend of a known
constant; this lemma is the Lean-side check that the rewrite
preserves `evalPure`).
-/

theorem constant_fold_extend_sound
    (e : Env) (value : Ref) (fromSz toSz : OpSize) (signed : Bool)
    (v : UInt64)
    (hv : e value = v) :
    evalPure e (.extend value fromSz toSz signed)
      = some (maskToSize
                (if signed then signExtend v fromSz
                           else maskToSize v fromSz)
                toSz) := by
  simp [evalPure, hv]

/-! ## F1-LN-014/015/016 — signed wide-op corner cases

Executable conformance checks pinning the signed `sMulHi` / `sDiv` /
`sMod` semantics to the ARM64-matching folds in `const_prop.cpp` —
the exact corner cases that motivated the former `sorry` placeholders:
divide-by-zero, `INT_MIN / -1`, and truncation toward zero.
`0x8000000000000000` is `INT64_MIN`; `0xFFFFFFFFFFFFFFFF` is `-1`. -/

-- sDiv: ARM64 sdiv x,0 = 0; INT_MIN/-1 wraps; truncation toward zero.
example : evalBinOp .sDiv 7 0 = 0 := by decide
example : evalBinOp .sDiv 0x8000000000000000 0xFFFFFFFFFFFFFFFF
            = 0x8000000000000000 := by decide
example : evalBinOp .sDiv 7 0xFFFFFFFFFFFFFFFE = 0xFFFFFFFFFFFFFFFD := by decide  -- 7 / -2 = -3
example : evalBinOp .sDiv 0xFFFFFFFFFFFFFFF9 2 = 0xFFFFFFFFFFFFFFFD := by decide  -- -7 / 2 = -3

-- sMod: divide-by-zero returns the dividend; INT_MIN%-1 = 0; sign of dividend.
example : evalBinOp .sMod 7 0 = 7 := by decide
example : evalBinOp .sMod 0x8000000000000000 0xFFFFFFFFFFFFFFFF = 0 := by decide
example : evalBinOp .sMod 7 0xFFFFFFFFFFFFFFFE = 1 := by decide                   -- 7 % -2 = 1
example : evalBinOp .sMod 0xFFFFFFFFFFFFFFF9 2 = 0xFFFFFFFFFFFFFFFF := by decide  -- -7 % 2 = -1

-- sMulHi: upper 64 bits of the signed 128-bit product.
example : evalBinOp .sMulHi 0xFFFFFFFFFFFFFFFF 0xFFFFFFFFFFFFFFFF = 0 := by decide          -- (-1)*(-1)=1
example : evalBinOp .sMulHi 0x8000000000000000 0x8000000000000000
            = 0x4000000000000000 := by decide                                              -- INT_MIN^2 = 2^126
example : evalBinOp .sMulHi 0xFFFFFFFFFFFFFFFF 1 = 0xFFFFFFFFFFFFFFFF := by decide          -- (-1)*1=-1

/-! ## F2-PS-ALG — algebraic_simplify pass soundness

Per-rewrite soundness for `core/src/passes/algebraic.cpp::try_simplify`,
which fires when exactly one operand is a known constant (the two-constant
case belongs to `constant_propagate`). Each lemma shows the rewrite to a
`Constant` preserves `evalPure`. Mirrors the F1-LN-010 per-op approach for
constant folding. -/

-- x - x -> 0 ;  x ^ x -> 0  (same-ref identities)
theorem algebraic_sub_self_sound (e : Env) (r : Ref) (sz : OpSize) :
    evalPure e (.binop .sub r r sz) = evalPure e (.constant 0 sz) := by
  simp [evalPure, evalBinOp]

theorem algebraic_xor_self_sound (e : Env) (r : Ref) (sz : OpSize) :
    evalPure e (.binop .xor r r sz) = evalPure e (.constant 0 sz) := by
  simp [evalPure, evalBinOp]

-- x * 0 -> 0  and  0 * x -> 0
theorem algebraic_mul_zero_r_sound (e : Env) (lhs rhs : Ref) (sz : OpSize)
    (h : e rhs = 0) :
    evalPure e (.binop .mul lhs rhs sz) = evalPure e (.constant 0 sz) := by
  simp [evalPure, evalBinOp, h]

theorem algebraic_mul_zero_l_sound (e : Env) (lhs rhs : Ref) (sz : OpSize)
    (h : e lhs = 0) :
    evalPure e (.binop .mul lhs rhs sz) = evalPure e (.constant 0 sz) := by
  simp [evalPure, evalBinOp, h]

-- x & 0 -> 0  and  0 & x -> 0
theorem algebraic_and_zero_r_sound (e : Env) (lhs rhs : Ref) (sz : OpSize)
    (h : e rhs = 0) :
    evalPure e (.binop .and lhs rhs sz) = evalPure e (.constant 0 sz) := by
  simp [evalPure, evalBinOp, h]

theorem algebraic_and_zero_l_sound (e : Env) (lhs rhs : Ref) (sz : OpSize)
    (h : e lhs = 0) :
    evalPure e (.binop .and lhs rhs sz) = evalPure e (.constant 0 sz) := by
  simp [evalPure, evalBinOp, h]

-- high half of x*0 = 0 (unsigned and signed)
theorem algebraic_umulhi_zero_r_sound (e : Env) (lhs rhs : Ref) (sz : OpSize)
    (h : e rhs = 0) :
    evalPure e (.binop .uMulHi lhs rhs sz) = evalPure e (.constant 0 sz) := by
  simp [evalPure, evalBinOp, h]

theorem algebraic_smulhi_zero_r_sound (e : Env) (lhs rhs : Ref) (sz : OpSize)
    (h : e rhs = 0) :
    evalPure e (.binop .sMulHi lhs rhs sz) = evalPure e (.constant 0 sz) := by
  simp [evalPure, evalBinOp, toSignedInt, h]

-- 0 / x -> 0 ; 0 % x -> 0 (signed)
theorem algebraic_sdiv_zero_l_sound (e : Env) (lhs rhs : Ref) (sz : OpSize)
    (h : e lhs = 0) :
    evalPure e (.binop .sDiv lhs rhs sz) = evalPure e (.constant 0 sz) := by
  simp [evalPure, evalBinOp, ofSignedInt, toSignedInt, h]

-- x % 1 -> 0 (signed)
theorem algebraic_smod_one_r_sound (e : Env) (lhs rhs : Ref) (sz : OpSize)
    (h : e rhs = 1) :
    evalPure e (.binop .sMod lhs rhs sz) = evalPure e (.constant 0 sz) := by
  simp [evalPure, evalBinOp, ofSignedInt, toSignedInt, h]

-- high half of 0*x = 0 (unsigned and signed)
theorem algebraic_umulhi_zero_l_sound (e : Env) (lhs rhs : Ref) (sz : OpSize)
    (h : e lhs = 0) :
    evalPure e (.binop .uMulHi lhs rhs sz) = evalPure e (.constant 0 sz) := by
  simp [evalPure, evalBinOp, h]

theorem algebraic_smulhi_zero_l_sound (e : Env) (lhs rhs : Ref) (sz : OpSize)
    (h : e lhs = 0) :
    evalPure e (.binop .sMulHi lhs rhs sz) = evalPure e (.constant 0 sz) := by
  simp [evalPure, evalBinOp, toSignedInt, h]

-- high half of x*1 = 0 (unsigned only; signed needs a range fact)
theorem algebraic_umulhi_one_r_sound (e : Env) (lhs rhs : Ref) (sz : OpSize)
    (h : e rhs = 1) :
    evalPure e (.binop .uMulHi lhs rhs sz) = evalPure e (.constant 0 sz) := by
  simp only [evalPure, evalBinOp, h]
  have hb : (e lhs).toNat / 18446744073709551616 = 0 :=
    Nat.div_eq_of_lt (e lhs).toNat_lt
  simp [hb]

-- x % 1 -> 0 (unsigned)
theorem algebraic_umod_one_r_sound (e : Env) (lhs rhs : Ref) (sz : OpSize)
    (h : e rhs = 1) :
    evalPure e (.binop .uMod lhs rhs sz) = evalPure e (.constant 0 sz) := by
  simp [evalPure, evalBinOp, h]

-- 0 / x -> 0 ; 0 % x -> 0 (unsigned) ; 0 % x -> 0 (signed)
theorem algebraic_udiv_zero_l_sound (e : Env) (lhs rhs : Ref) (sz : OpSize)
    (h : e lhs = 0) :
    evalPure e (.binop .uDiv lhs rhs sz) = evalPure e (.constant 0 sz) := by
  simp [evalPure, evalBinOp, h]

theorem algebraic_umod_zero_l_sound (e : Env) (lhs rhs : Ref) (sz : OpSize)
    (h : e lhs = 0) :
    evalPure e (.binop .uMod lhs rhs sz) = evalPure e (.constant 0 sz) := by
  simp [evalPure, evalBinOp, h]

theorem algebraic_smod_zero_l_sound (e : Env) (lhs rhs : Ref) (sz : OpSize)
    (h : e lhs = 0) :
    evalPure e (.binop .sMod lhs rhs sz) = evalPure e (.constant 0 sz) := by
  simp [evalPure, evalBinOp, ofSignedInt, toSignedInt, h]

-- x | -1 -> -1  and  -1 | x -> -1, where -1 is the size's all-ones value.
theorem algebraic_or_allones_r_sound (e : Env) (lhs rhs : Ref) (sz : OpSize)
    (h : e rhs = maskToSize 0xFFFFFFFFFFFFFFFF sz) :
    evalPure e (.binop .or lhs rhs sz)
      = evalPure e (.constant 0xFFFFFFFFFFFFFFFF sz) := by
  cases sz <;>
    simp only [evalPure, evalBinOp, maskToSize, h, Option.some.injEq] <;>
    bv_decide

theorem algebraic_or_allones_l_sound (e : Env) (lhs rhs : Ref) (sz : OpSize)
    (h : e lhs = maskToSize 0xFFFFFFFFFFFFFFFF sz) :
    evalPure e (.binop .or lhs rhs sz)
      = evalPure e (.constant 0xFFFFFFFFFFFFFFFF sz) := by
  cases sz <;>
    simp only [evalPure, evalBinOp, maskToSize, h, Option.some.injEq] <;>
    bv_decide

end PrismaIR
