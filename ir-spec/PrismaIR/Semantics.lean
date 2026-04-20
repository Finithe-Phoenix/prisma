/-
  PrismaIR.Semantics — operational semantics of the Prisma IR.

  Status: VERY EARLY SKETCH. What's here is a toy "step" relation for the
  pure fragment (constants, binops). Memory, control flow, and the TSO
  variants are deliberately left unspecified in this Fase 0 preview — they
  need a proper memory model, which is a design task for Fase 1 weeks 9-14.

  The eventual goal is:

    * `MachineState` — register file, memory with a weak-memory semantics
      that captures both TSO (for non-TSO-rewritten ops) and relaxed ARM
      semantics (for ops the adaptive pass has rewritten away).

    * `step : MachineState → Stmt → Option MachineState × Option (Ref × UInt64)`
      — partial function describing one step of execution, returning an
      updated state and optionally a (ref, value) pair to extend the SSA env.

    * `Refines : Function → Function → Prop` — a pass is sound iff every
      observable trace of the transformed function is a trace of the
      original. This is the property each optimization pass must prove.
-/

import PrismaIR.Syntax

namespace PrismaIR

/-- Evaluate a pure binary operation on two 64-bit values. Size-specific
    masking is applied afterwards. This ignores flag production for now;
    flags live in the full semantics later. -/
def evalBinOp (op : BinOp) (lhs rhs : UInt64) : UInt64 :=
  match op with
  | .add => lhs + rhs
  | .sub => lhs - rhs
  | .and => lhs &&& rhs
  | .or  => lhs ||| rhs
  | .xor => lhs ^^^ rhs
  | .shl => lhs <<< (rhs &&& 0x3F)
  | .shr => lhs >>> (rhs &&& 0x3F)
  | .sar => lhs >>> (rhs &&& 0x3F)  -- TODO: proper signed shift
  | .rol =>
      let n := rhs &&& 0x3F
      if n == 0 then lhs else (lhs <<< n) ||| (lhs >>> (64 - n))
  | .ror =>
      let n := rhs &&& 0x3F
      if n == 0 then lhs else (lhs >>> n) ||| (lhs <<< (64 - n))

/-- Mask a 64-bit value down to `size` bits. -/
def maskToSize (v : UInt64) : OpSize → UInt64
  | .i8  => v &&& 0xFF
  | .i16 => v &&& 0xFFFF
  | .i32 => v &&& 0xFFFFFFFF
  | .i64 => v

/-- A toy environment: Ref → UInt64 (undefined refs get 0). The real semantics
    will use `Option UInt64` or a more structured store. -/
abbrev Env := Ref → UInt64

/-- Extend an env with a new binding. -/
def Env.extend (e : Env) (r : Ref) (v : UInt64) : Env :=
  fun r' => if r' = r then v else e r'

/-- Evaluate the pure fragment of the IR inside a given environment. Returns
    `none` for ops that are not pure (memory, control flow) — those are
    handled by the step relation in the full semantics. -/
def evalPure (e : Env) : Op → Option UInt64
  | .constant v sz          => some (maskToSize v sz)
  | .binop op lhs rhs sz    => some (maskToSize (evalBinOp op (e lhs) (e rhs)) sz)
  | _                        => none

/-- Smoke test — demonstrates Lean 4 + Prisma IR compile together. -/
def exampleProgram : Function :=
  { blocks :=
      [{ id := 0
       , stmts :=
           [ { result := some 0, op := .constant 10 .i64 }
           , { result := some 1, op := .constant 32 .i64 }
           , { result := some 2, op := .binop .add 0 1 .i64 }
           , { result := none, op := .ret }
           ]
       }]
    entry := 0
  }

/-- Verify our toy evaluator on the example program. -/
example : evalPure (fun _ => 0) (.constant 42 .i64) = some 42 := by decide

example :
    let e₀ : Env := fun _ => 0
    let e₁ := e₀.extend 0 10
    let e₂ := e₁.extend 1 32
    evalPure e₂ (.binop .add 0 1 .i64) = some 42 := by
  decide

end PrismaIR
