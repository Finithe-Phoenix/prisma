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


/-! ### Access-downgrade worked example

A concrete two-core state: core 0 has a pending store to address 5, core 1 owns
the write-private address 9. The downgrade theorems are exercised by computation
(`decide`/`rfl`), confirming core 0's whole barrier cannot disturb core 1's view
of its private address. -/

/-- Core 0 buffers `(5 := 1)`; core 1 has nothing buffered. Memory is all zero. -/
def twoCoreState : TSO :=
  { mem := fun _ => 0, sb := fun t => if t = 0 then [(5, 1)] else [] }

/-- Address 9 is write-private to core 1: core 0 (the only other core with a
    buffered store) never targets it. -/
example : NoStoreTo (twoCoreState.sb 0) 9 := by unfold NoStoreTo; decide

/-- A full barrier on core 0 leaves core 1's load of its private address 9
    unchanged — a computed instance of `load_private_fence_other`. -/
example : (twoCoreState.fence 0).load 1 9 = twoCoreState.load 1 9 := by rfl

/-- And that value is 0 (untouched memory), even though the barrier did publish
    core 0's store to address 5. -/
example : (twoCoreState.fence 0).load 1 9 = 0 := by rfl

/-- The barrier really did publish core 0's own store (5 := 1) — the elision
    soundness is non-trivial: the barrier has a visible effect elsewhere, just
    not on the private address. -/
example : (twoCoreState.fence 0).load 0 5 = 1 := by rfl
end TSO
end PrismaIR
