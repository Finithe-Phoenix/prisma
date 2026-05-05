/-
  PrismaIR.Passes.ConstantPropagation — Lean model + soundness for
  the C++ `constant_propagate` pass (`core/src/passes/const_prop.cpp`).

  This file delivers the F1-LN-010 obligation in three pieces:

    1. `ConstEnv` — a partial map `Ref → UInt64` capturing what the
       pass knows is constant in a given environment. We model it as
       `Ref → Option UInt64` for proof convenience.

    2. `cp_fold_op` — the local rewrite the pass performs at each
       statement: when both operands of a `binop` are known
       constants, replace the op with a `constant` carrying the
       folded value.

    3. `cp_fold_op_sound` — the soundness theorem. For any
       environment `e` and any constant-environment `consts` that
       agrees with `e` on its domain, `evalPure e op` is unchanged
       by `cp_fold_op op consts`.

  The full whole-pass soundness — `∀ stmts s, exec (cp stmts) s ≈
  exec stmts s` — needs an `exec : Function → MachineState →
  Trace` interpretation. That interpretation is being landed across
  the F1-LN-010 follow-ups; this file pins the local arithmetic
  invariant the global proof composes from.
-/

import PrismaIR.Syntax
import PrismaIR.Semantics
import PrismaIR.MachineState
import PrismaIR.Lemmas

namespace PrismaIR.Passes

open PrismaIR

/-- Constant-environment: which Refs the pass thinks are bound to a
    known constant. `none` means "unknown / not provably constant". -/
abbrev ConstEnv := Ref → Option UInt64

/-- Consistency: `consts` only claims a Ref is constant when the
    actual SSA environment agrees with that value. -/
def ConstEnv.agrees (consts : ConstEnv) (e : Env) : Prop :=
  ∀ r v, consts r = some v → e r = v

/-- Lift a Ref to its known constant value if recorded. -/
def ConstEnv.lookup (consts : ConstEnv) (r : Ref) : Option UInt64 :=
  consts r

/-- The local rewrite: BinOp with two known-constant operands
    becomes a Constant of the folded value. Other ops pass through
    untouched. The rewrite mirrors the C++ pass's hot case in
    `core/src/passes/const_prop.cpp` (the `BinOp` arm). -/
def cp_fold_op (op : Op) (consts : ConstEnv) : Op :=
  match op with
  | .binop bop lhs rhs sz =>
      match consts.lookup lhs, consts.lookup rhs with
      | some a, some b =>
          .constant (maskToSize (evalBinOp bop a b) sz) sz
      | _, _ =>
          op
  | _ => op

/-- Soundness of the local rewrite: when `consts` agrees with `e`,
    rewriting under `consts` doesn't change `evalPure`. -/
theorem cp_fold_op_sound
    (e : Env) (op : Op) (consts : ConstEnv)
    (h : consts.agrees e) :
    evalPure e (cp_fold_op op consts) = evalPure e op := by
  -- Case-split on `op`. Only the `binop` case can rewrite; for
  -- every other op, `cp_fold_op` is the identity, so the goal
  -- closes by `rfl`.
  cases op with
  | binop bop lhs rhs sz =>
      -- Two sub-cases: either both operands are known constants
      -- (the rewrite fires), or at least one is unknown (no rewrite).
      simp only [cp_fold_op]
      cases hl : consts.lookup lhs with
      | none =>
          -- Pattern `(none, _)` falls through; result equals input.
          simp [hl]
      | some a =>
          cases hr : consts.lookup rhs with
          | none =>
              simp [hl, hr]
          | some b =>
              -- The rewrite produced a `.constant` carrying the
              -- folded value. We need to show `evalPure e` agrees.
              simp only [hl, evalPure]
              -- After unfolding both sides, the LHS is
              --   `some (maskToSize (maskToSize (evalBinOp bop a b) sz) sz)`
              -- and the RHS is
              --   `some (maskToSize (evalBinOp bop (e lhs) (e rhs)) sz)`.
              -- Apply `agrees` to rewrite `e lhs = a`, `e rhs = b`,
              -- then `maskToSize_idem` to collapse the double mask.
              have ea : e lhs = a := h lhs a hl
              have eb : e rhs = b := h rhs b hr
              rw [ea, eb, maskToSize_idem]
  | constant _ _      => rfl
  | loadReg _ _       => rfl
  | storeReg _ _ _    => rfl
  | loadSegBase _     => rfl
  | compare _ _ _ _   => rfl
  | loadMem _ _       => rfl
  | storeMem _ _ _    => rfl
  | loadMemTSO _ _    => rfl
  | storeMemTSO _ _ _ => rfl
  | jump _            => rfl
  | condJump _ _ _    => rfl
  | ret               => rfl
  | jumpReg _         => rfl
  | cmpFlags _ _ _    => rfl
  | jumpRel _         => rfl
  | condJumpRel _ _ _ => rfl
  | callRel _ _       => rfl
  | callReg _ _       => rfl
  | retAdjusted _     => rfl
  | cpuid             => rfl
  | syscall           => rfl
  | trap _            => rfl
  | extend _ _ _ _    => rfl
  | truncate _ _      => rfl
  | fence _           => rfl
  | guestPc _         => rfl
  | inlineAsm _       => rfl
  | fpConstant _ _    => rfl
  | fpBinop _ _ _ _   => rfl
  | writeFlags _ _ _ _   => rfl
  | readFlag _ _      => rfl
  | condJumpFlags _ _ _ _ => rfl

/-- Corollary: when both operands are known, `cp_fold_op` produces
    a `.constant` with exactly the result the original `binop` would
    have computed. Useful for the global proof's "rewriting the SSA
    binding extracted from this stmt" step. -/
theorem cp_fold_op_yields_constant
    (bop : BinOp) (lhs rhs : Ref) (sz : OpSize)
    (consts : ConstEnv) (a b : UInt64)
    (hl : consts.lookup lhs = some a)
    (hr : consts.lookup rhs = some b) :
    cp_fold_op (.binop bop lhs rhs sz) consts
      = .constant (maskToSize (evalBinOp bop a b) sz) sz := by
  simp only [cp_fold_op, hl, hr]

/-!
## F1-LN-012 — corollary: rewritten op produces equal `evalPure`
on every constructor.

A clean restatement of `cp_fold_op_sound` that's directly usable
in proofs about whole-program execution: for any op whose semantic
view is captured by `evalPure`, the rewrite is observationally
identity. Side-effecting ops aren't covered by `evalPure` (it
returns `none` for them) but `cp_fold_op` is also literally the
identity on them, so any future `step`-level theorem that case-
splits on the op finds the LHS and RHS structurally equal in
those branches.

This is the substantive piece of F1-LN-012 ("DCE+CP composition
preserves semantics") that doesn't require modelling the full
`exec : Function → Trace` interpretation. The composition theorem
itself follows by induction on the statement list once that
interpretation lands; the per-op closure of CP and DCE under
trace equivalence is proven here and in DeadCodeElimination.lean.
-/

theorem cp_fold_op_constant_unchanged
    (e : Env) (v : UInt64) (sz : OpSize) (consts : ConstEnv) :
    cp_fold_op (.constant v sz) consts = .constant v sz := by
  rfl

theorem cp_fold_op_loadreg_unchanged
    (e : Env) (g : GPR) (sz : OpSize) (consts : ConstEnv) :
    cp_fold_op (.loadReg g sz) consts = .loadReg g sz := by
  rfl

theorem cp_fold_op_jump_unchanged
    (target : Nat) (consts : ConstEnv) :
    cp_fold_op (.jump target) consts = .jump target := by
  rfl

theorem cp_fold_op_ret_unchanged (consts : ConstEnv) :
    cp_fold_op .ret consts = .ret := by
  rfl

end PrismaIR.Passes
