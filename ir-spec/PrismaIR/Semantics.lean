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

/-- Two's-complement reinterpretation of a 64-bit word as a signed integer
    in `[-2^63, 2^63)`. -/
def toSignedInt (a : UInt64) : Int :=
  if a.toNat < 2 ^ 63 then (a.toNat : Int) else (a.toNat : Int) - (2 ^ 64 : Int)

/-- Wrap a signed integer back into a 64-bit word (reduce mod `2^64`). -/
def ofSignedInt (i : Int) : UInt64 :=
  (i % (2 ^ 64 : Int)).toNat.toUInt64

/-- Evaluate a pure binary operation on two 64-bit values. Size-specific
    masking is applied afterwards. This ignores flag production for now;
    flags live in the full semantics later. -/
def evalBinOp (op : BinOp) (lhs rhs : UInt64) : UInt64 :=
  match op with
  | .add => lhs + rhs
  | .sub => lhs - rhs
  | .mul => lhs * rhs
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
  | .rcl =>
      let n := rhs &&& 0x3F
      if n == 0 then lhs else (lhs <<< n) ||| (lhs >>> (64 - n))
  | .rcr =>
      let n := rhs &&& 0x3F
      if n == 0 then lhs else (lhs >>> n) ||| (lhs <<< (64 - n))
  -- F2-BK-007 — wide-form ops. Both unsigned (Nat) and signed (Int)
  -- variants are now concrete. The signed corner cases mirror the
  -- ARM64-matching folds in `const_prop.cpp` exactly: at this pure
  -- pre-trap eval level x86's `#DE` does not exist, so divide-by-zero
  -- and `INT_MIN / -1` follow ARM64 `sdiv`/`msub` (see docs/rfc/0012),
  -- not x86 trap semantics. Trap injection is a runtime concern layered
  -- on top of this total function.
  | .uMulHi =>
      let prod : Nat := lhs.toNat * rhs.toNat
      (prod / (2 ^ 64)).toUInt64
  | .uDiv =>
      if rhs == 0 then 0 else lhs / rhs
  | .uMod =>
      if rhs == 0 then lhs else lhs % rhs
  | .sMulHi =>
      -- Upper 64 bits of the two's-complement 128-bit signed product.
      let prod : Int := toSignedInt lhs * toSignedInt rhs
      let prodU : Nat := (prod % (2 ^ 128 : Int)).toNat
      (prodU / 2 ^ 64).toUInt64
  | .sDiv =>
      let sa : Int := toSignedInt lhs
      let sb : Int := toSignedInt rhs
      if sb == 0 then 0                                    -- ARM64 sdiv x, 0 = 0
      else if sa == -(2 ^ 63 : Int) && sb == -1 then lhs   -- INT_MIN / -1 wraps to INT_MIN
      else ofSignedInt (Int.tdiv sa sb)                    -- truncate toward zero (C `/`)
  | .sMod =>
      let sa : Int := toSignedInt lhs
      let sb : Int := toSignedInt rhs
      if sb == 0 then lhs                                  -- ARM msub: a - 0*b = a
      else if sa == -(2 ^ 63 : Int) && sb == -1 then 0     -- remainder of INT_MIN / -1 is 0
      else ofSignedInt (Int.tmod sa sb)                    -- truncate toward zero (C `%`)

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

/-- Evaluate a comparison condition over two 64-bit values, returning a
    1-bit result (1 = true, 0 = false). The masking to `size` is the
    caller's job; for the purposes of CondCode evaluation only the
    low bits matter for the unsigned comparisons, and the sign-bit
    interpretation matters for the signed ones. -/
def evalCompare (code : CondCode) (lhs rhs : UInt64) : UInt64 :=
  -- Signed comparison via the toInt64 view; Lean's UInt64 has a
  -- two's-complement Int64 reading.
  let lhs_s := lhs.toInt64
  let rhs_s := rhs.toInt64
  match code with
  | CondCode.eq    => if lhs == rhs then 1 else 0
  | CondCode.ne    => if lhs != rhs then 1 else 0
  | CondCode.ult   => if lhs < rhs then 1 else 0
  | CondCode.ule   => if lhs ≤ rhs then 1 else 0
  | CondCode.ugt   => if lhs > rhs then 1 else 0
  | CondCode.uge   => if lhs ≥ rhs then 1 else 0
  | CondCode.slt   => if lhs_s < rhs_s then 1 else 0
  | CondCode.sle   => if lhs_s ≤ rhs_s then 1 else 0
  | CondCode.sgt   => if lhs_s > rhs_s then 1 else 0
  | CondCode.sge   => if lhs_s ≥ rhs_s then 1 else 0
  | _              => 0  -- flag-direct codes are NZCV-driven; not modelled here yet

/-- Sign-extend the low `n` bits of `v` to a 64-bit value. -/
def signExtend (v : UInt64) : OpSize → UInt64
  | .i8  =>
      let low := v &&& 0xFF
      if (low &&& 0x80) == 0 then low else low ||| 0xFFFFFFFFFFFFFF00
  | .i16 =>
      let low := v &&& 0xFFFF
      if (low &&& 0x8000) == 0 then low else low ||| 0xFFFFFFFFFFFF0000
  | .i32 =>
      let low := v &&& 0xFFFFFFFF
      if (low &&& 0x80000000) == 0 then low else low ||| 0xFFFFFFFF00000000
  | .i64 => v

/-- Evaluate the pure fragment of the IR inside a given environment. Returns
    `none` for ops that are not pure (memory, control flow, side effects).
    Pure ops grow over time: F1-LN-004 added Compare, Extend, Truncate. -/
def evalPure (e : Env) : Op → Option UInt64
  | .constant v sz                     => some (maskToSize v sz)
  | .binop op lhs rhs sz               =>
      some (maskToSize (evalBinOp op (e lhs) (e rhs)) sz)
  | .compare cc lhs rhs _              => some (evalCompare cc (e lhs) (e rhs))
  | .extend value fromSz toSz signed   =>
      some (maskToSize
              (if signed then signExtend (e value) fromSz
                         else maskToSize (e value) fromSz)
              toSz)
  | .truncate value toSz               => some (maskToSize (e value) toSz)
  | _                                   => none

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
