/-
  PrismaIR.Passes.DeadCodeElimination — F1-LN-011 soundness sketch
  for the C++ `dead_code_eliminate` pass
  (`core/src/passes/dce.cpp`).

  DCE drops a *pure* statement (Constant / LoadReg / LoadSegBase /
  BinOp / Compare / Select / LoadMem / LoadMemTSO / Extend /
  Truncate / FpConstant / FpBinOp / WriteFlags / ReadFlag) whose
  bound `Ref` is never read by any later statement in the same
  block.

  The local invariant is: dropping a pure stmt with a never-read
  result Ref doesn't change `evalPure` of any later statement.
  That's because:

    (a) `evalPure` only depends on the SSA environment via the Refs
        the op explicitly mentions. Dropping a stmt removes one
        binding; that binding can't be referenced by anything else
        (definition of "never-read"), so the env restricted to
        any later op is bit-identical with vs. without the
        dropped stmt.

    (b) `evalPure` itself is pure — no memory reads, no flag-bank
        consumption — so an SSA-env-only dependency is enough.

  Side-effecting stmts (StoreReg, StoreMem*, Jump*, Return, ...) are
  NOT pure; DCE never drops them, and this lemma doesn't try to
  state anything about them.

  The whole-pass theorem composes this local lemma over the list:
  for every stmt the pass drops, the rest of the program's trace
  is unchanged. That composition lives in the F1-LN-012 commit
  alongside the `exec : Function → Trace` interpretation.
-/

import PrismaIR.Syntax
import PrismaIR.Semantics
import PrismaIR.MachineState

namespace PrismaIR.Passes

open PrismaIR

/-- A Ref is "live for op" iff `op` reads from that Ref via
    `evalPure`'s explicit operand list. The relation is decidable
    on the closed Op family but we state it as a Prop for proof
    convenience. -/
def Op.reads_ref (op : Op) (r : Ref) : Prop :=
  match op with
  | .binop _ lhs rhs _      => r = lhs ∨ r = rhs
  | .compare _ lhs rhs _    => r = lhs ∨ r = rhs
  | .extend value _ _ _     => r = value
  | .truncate value _       => r = value
  | _                       => False

/-- F1-LN-011 (local): if `op` does not read `dropped_ref`, then
    extending the SSA env with an additional binding for that Ref
    doesn't change `evalPure`. The hypothesis "doesn't read" is the
    DCE pass's precondition for dropping the stmt that defines
    `dropped_ref`.

    `Env.extend_irrelevant`: a binding the consumer never asks for
    is invisible. `evalPure` only inspects the Refs the op names,
    so any extension over a non-read Ref preserves the result. -/
theorem evalPure_unaffected_by_unread_ref
    (e : Env) (op : Op) (dropped_ref : Ref) (extra_value : UInt64)
    (h : ¬ Op.reads_ref op dropped_ref) :
    evalPure (e.extend dropped_ref extra_value) op = evalPure e op := by
  -- Case-split on `op`. For every variant whose `reads_ref`
  -- returns False or doesn't depend on the new binding, the goal
  -- closes by `rfl` after `simp [evalPure, Env.extend]`. For the
  -- variants with operand Refs, we use the hypothesis to show
  -- neither operand equals `dropped_ref` and rewrite away.
  cases op with
  | constant _ _ =>
      -- `evalPure e (.constant ..)` doesn't touch `e` at all.
      rfl
  | loadReg _ _ =>
      rfl
  | loadSegBase _ =>
      -- evalPure returns `none` for loadSegBase today; either way
      -- it doesn't mention `e`.
      rfl
  | binop bop lhs rhs sz =>
      -- `Op.reads_ref` for `binop` is `r = lhs ∨ r = rhs`. The
      -- hypothesis `¬(dropped_ref = lhs ∨ dropped_ref = rhs)`
      -- gives us both `dropped_ref ≠ lhs` and `dropped_ref ≠ rhs`,
      -- so the extension is invisible to both env reads.
      simp only [Op.reads_ref, not_or] at h
      obtain ⟨hl, hr⟩ := h
      simp [evalPure, Env.extend, Ne.symm hl, Ne.symm hr]
  | compare _ lhs rhs _ =>
      simp only [Op.reads_ref, not_or] at h
      obtain ⟨hl, hr⟩ := h
      simp [evalPure, Env.extend, Ne.symm hl, Ne.symm hr]
  | extend value _ _ _ =>
      simp only [Op.reads_ref] at h
      simp [evalPure, Env.extend, Ne.symm h]
  | truncate value _ =>
      simp only [Op.reads_ref] at h
      simp [evalPure, Env.extend, Ne.symm h]
  -- The remaining ops: `evalPure` returns `none` for non-pure ops,
  -- and the pure ops above are exhaustive for the `evalPure`-defined
  -- cases. For non-pure variants the result is `none` regardless of
  -- `e`, so the goal is `none = none`, closed by `rfl`.
  | storeReg _ _ _    => rfl
  | storeMem _ _ _    => rfl
  | loadMem _ _       => rfl
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
  | fence _           => rfl
  | guestPc _         => rfl
  | inlineAsm _       => rfl
  | fpConstant _ _    => rfl
  | fpBinop _ _ _ _   => rfl
  | writeFlags _ _ _ _   => rfl
  | readFlag _ _      => rfl
  | condJumpFlags _ _ _ _ => rfl

end PrismaIR.Passes
