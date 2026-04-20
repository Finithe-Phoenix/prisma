/-
  PrismaIR.MachineState — toy machine state for the pure IR fragment.

  Status: minimal. The full semantics (Fase 1 weeks 9-14) will extend this
  to a proper weak-memory model that distinguishes TSO and non-TSO loads /
  stores, models threads, and supports reasoning about the TSO-adaptive
  rewrite (Pillar 3).

  What we have now:
    * A register file indexed by GPR, holding UInt64 values.
    * An SSA environment Env from `Syntax` (Ref → UInt64).
    * No memory, no threading, no observable events.

  This is already enough to state and prove trivial properties of `evalPure`.
-/

import PrismaIR.Syntax
import PrismaIR.Semantics

namespace PrismaIR

/-- A toy register file: a total function from GPR to UInt64. Undefined
    registers read zero. The real semantics will make reads explicit in the
    step relation. -/
abbrev RegFile := GPR → UInt64

/-- Update a register in the file. -/
def RegFile.set (rf : RegFile) (r : GPR) (v : UInt64) : RegFile :=
  fun r' => if r' = r then v else rf r'

/-- A toy machine state: register file and SSA environment only. Memory and
    thread state come later. -/
structure MachineState where
  regs : RegFile
  env  : Env
  deriving Inhabited

/-- The "empty" state: all zeroes. -/
def MachineState.empty : MachineState :=
  { regs := fun _ => 0, env := fun _ => 0 }

end PrismaIR
