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

A one-line statement that `maskToSize (maskToSize v sz) sz = maskToSize v sz`.
The proof requires AND idempotence on `UInt64`, which lives in mathlib.
We mark it pending until mathlib is wired into the Lake build (planned
for Fase 1 week 9 alongside full memory-model work).
-/

theorem maskToSize_idem (v : UInt64) (sz : OpSize) :
    maskToSize (maskToSize v sz) sz = maskToSize v sz := by
  sorry

end PrismaIR
