/-
  PrismaIR.Trace — F1-LN-013 observable-trace definition.

  An "observable trace" is the externally-visible side-effect record of
  a program's execution. Two programs are observably equivalent (`≈`)
  iff every execution of one produces a trace that some execution of
  the other can also produce. This is the property each optimisation
  pass must preserve: a sound pass can rearrange / drop / fold
  internal computation, but cannot change what an outside observer
  sees.

  What counts as observable in the Prisma model:

    * Memory writes (`StoreMem` / `StoreMemTSO`). Every byte written to
      a guest address is visible to other threads / the host kernel,
      so the timing and order of these writes IS the program's
      external behaviour.
    * Register-file final state. The end-of-block guest GPR contents
      flow back into the dispatcher and from there to syscall arguments
      / return values, so they're observable too.
    * Control flow termination kind: did the program return normally
      (Return), branch out via JumpRel, raise a Trap, etc. The
      dispatcher branches on this; an observer can tell.

  What is NOT observable (and therefore free for optimisation):

    * Intermediate SSA values bound to Refs. Pure ops can be folded,
      reordered, deduplicated.
    * The number of stmts. DCE / CSE / peephole all change this; none
      are unsound by virtue of doing so.

  This module defines `Event`, `Trace`, and the equivalence relation
  `Trace.observable_equiv`. It deliberately does NOT yet wire the
  trace generator to the `step` relation in `MachineState.lean`;
  the wiring is F1-LN-010 (constant_propagate soundness) territory.
-/

import PrismaIR.Syntax
import PrismaIR.MachineState

namespace PrismaIR

/-- How a block ended. The dispatcher distinguishes these to decide
    what to do next. -/
inductive TerminationKind where
  | normal                                       -- Return
  | tailJump      (guestPc : UInt64)              -- JumpRel
  | conditional   (guestPc fallthrough : UInt64)  -- CondJumpRel chose taken
  | trap          (kind : TrapKind)               -- INT3 / UD2 / DE
  | halt                                          -- Cpuid / Syscall placeholder
  deriving Repr

/-- One observable event. The list of events a program emits during
    execution is its trace. -/
inductive Event where
  | memWrite     (addr : Nat) (value : UInt64) (size : OpSize) : Event
  | regSnapshot  (regs : RegFile)                              : Event
  | terminated   (kind : TerminationKind)                      : Event
  deriving Inhabited

/-- A trace is a list of events. The first executed event is at the
    head; concatenation `++` is sequential composition. -/
abbrev Trace := List Event

namespace Trace

/-- The empty trace — a program that did nothing observable. -/
def empty : Trace := []

/-- Append an event to the end. Used by the future trace generator
    (`exec : Function → MachineState → Trace`). -/
def snoc (t : Trace) (e : Event) : Trace := t ++ [e]

/-- Observable equivalence. Two traces are observably equivalent iff
    they are equal as Lists of Events. We keep the relation explicit
    even though it currently equals `=` so future weak-memory
    refinement can relax it (e.g. allow reordering of independent
    `memWrite` events under TSO).

    A pass `pass : Function → Function` is sound iff for every
    initial state `s`, `exec (pass f) s ≈ exec f s`. F1-LN-010
    discharges this for `constant_propagate`; F1-LN-011 for
    `dead_code_eliminate`; F1-LN-012 for their composition. -/
def observable_equiv (a b : Trace) : Prop := a = b

/-- The relation is reflexive, symmetric, and transitive — it's
    just `=` with a wrapper for now. -/

theorem observable_equiv_refl (t : Trace) : observable_equiv t t := rfl

theorem observable_equiv_symm
    {a b : Trace} (h : observable_equiv a b) :
    observable_equiv b a := h.symm

theorem observable_equiv_trans
    {a b c : Trace} (h₁ : observable_equiv a b) (h₂ : observable_equiv b c) :
    observable_equiv a c := h₁.trans h₂

end Trace

end PrismaIR
