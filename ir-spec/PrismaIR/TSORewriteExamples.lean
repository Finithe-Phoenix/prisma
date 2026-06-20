import PrismaIR.Syntax
import PrismaIR.TSO
import PrismaIR.TSORewrite

/-
  PrismaIR.TSORewriteExamples — F1-LN-016 worked examples.

  Runs the barrier-elimination machinery on a concrete store->fence->load block,
  so the M1-M4 definitions are exercised by `rfl` (definitional computation), not
  just proved in the abstract. These catch a whole class of definitional bug
  (a `proj`/`elimFences`/`filterMap` that typechecks but computes wrong) the
  theorems alone would not surface.
-/

namespace PrismaIR
namespace TSO

/-- A concrete translated block: buffer a TSO store, a full barrier, then a TSO
    load of the same guest location — the store->fence->load the adaptive pass
    targets once it proves the location thread-local. -/
def exampleBlock : List Op :=
  [Op.storeMemTSO 0 1 OpSize.i64, Op.fence FenceKind.mfence, Op.loadMemTSO 0 OpSize.i64]

/-- The barrier is the only fence, so `elimFences` drops exactly it. -/
example : elimFences exampleBlock
    = [Op.storeMemTSO 0 1 OpSize.i64, Op.loadMemTSO 0 OpSize.i64] := rfl

/-- The projected original carries the bar between the store and the load. -/
example (ρ : Ref → Addr) (σ : Ref → Val) :
    projBlock ρ σ exampleBlock
      = [MOp.store (ρ 0) (σ 1), MOp.bar, MOp.load (ρ 0)] := rfl

/-- Under any environment the rewrite deletes the bar and keeps the store/load
    in program order — a computed instance of `projBlock_elimFences`. -/
example (ρ : Ref → Addr) (σ : Ref → Val) :
    projBlock ρ σ (elimFences exampleBlock)
      = [MOp.store (ρ 0) (σ 1), MOp.load (ρ 0)] := rfl

/-- The eliminated block is bar-free on this concrete input (the capstone). -/
example (ρ : Ref → Addr) (σ : Ref → Val) :
    ∀ m ∈ projBlock ρ σ (elimFences exampleBlock), isBar m = false :=
  elimFences_barFree ρ σ exampleBlock

end TSO
end PrismaIR
