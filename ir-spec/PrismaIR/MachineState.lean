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

/-- F1-LN-005: byte-addressed memory as a total function `Nat → UInt8`.
    Undefined addresses read zero; the real weak-memory semantics
    refines this with TSO / non-TSO observability. -/
abbrev Memory := Nat → UInt8

/-- Update a single byte in memory. -/
def Memory.set (m : Memory) (addr : Nat) (b : UInt8) : Memory :=
  fun a => if a = addr then b else m a

/-- Read a 64-bit value from memory in little-endian order. -/
def Memory.read64 (m : Memory) (addr : Nat) : UInt64 :=
  let b0 : UInt64 := (m addr).toUInt64
  let b1 : UInt64 := (m (addr + 1)).toUInt64
  let b2 : UInt64 := (m (addr + 2)).toUInt64
  let b3 : UInt64 := (m (addr + 3)).toUInt64
  let b4 : UInt64 := (m (addr + 4)).toUInt64
  let b5 : UInt64 := (m (addr + 5)).toUInt64
  let b6 : UInt64 := (m (addr + 6)).toUInt64
  let b7 : UInt64 := (m (addr + 7)).toUInt64
  b0 ||| (b1 <<< 8) ||| (b2 <<< 16) ||| (b3 <<< 24)
     ||| (b4 <<< 32) ||| (b5 <<< 40) ||| (b6 <<< 48) ||| (b7 <<< 56)

/-- Write a 64-bit value to memory in little-endian order. -/
def Memory.write64 (m : Memory) (addr : Nat) (v : UInt64) : Memory :=
  let m0 := m.set addr        (v &&& 0xFF).toUInt8
  let m1 := m0.set (addr + 1) ((v >>> 8) &&& 0xFF).toUInt8
  let m2 := m1.set (addr + 2) ((v >>> 16) &&& 0xFF).toUInt8
  let m3 := m2.set (addr + 3) ((v >>> 24) &&& 0xFF).toUInt8
  let m4 := m3.set (addr + 4) ((v >>> 32) &&& 0xFF).toUInt8
  let m5 := m4.set (addr + 5) ((v >>> 40) &&& 0xFF).toUInt8
  let m6 := m5.set (addr + 6) ((v >>> 48) &&& 0xFF).toUInt8
  m6.set (addr + 7) ((v >>> 56) &&& 0xFF).toUInt8

/-- F1-LN-007 — concrete EFLAGS carrier. Six bits matching the C++
    `FlagBit` enum (carry, zero, sign, overflow, parity, aux). The
    "no flags written yet" state has every bit zero. -/
structure Flags where
  carry    : Bool := false
  zero     : Bool := false
  sign     : Bool := false
  overflow : Bool := false
  parity   : Bool := false
  aux      : Bool := false
  deriving Inhabited, Repr, BEq

/-- Compute Flags from a `WriteFlags`-style binop. Mirrors the
    minimal subset the lowerer supports today (Sub / Add / And).
    For ops not in the subset, returns the all-zero Flags. -/
def Flags.fromBinop (op : BinOp) (lhs rhs : UInt64) : Flags :=
  match op with
  | .sub =>
      let r := lhs - rhs
      { carry    := lhs < rhs
      , zero     := r == 0
      , sign     := (r >>> 63) == 1
      , overflow := false   -- TODO: signed overflow detection
      , parity   := false
      , aux      := false }
  | .add =>
      let r := lhs + rhs
      { carry    := r < lhs
      , zero     := r == 0
      , sign     := (r >>> 63) == 1
      , overflow := false
      , parity   := false
      , aux      := false }
  | .and =>
      let r := lhs &&& rhs
      { carry    := false
      , zero     := r == 0
      , sign     := (r >>> 63) == 1
      , overflow := false
      , parity   := false
      , aux      := false }
  | _    => {}

/-- Read a single flag bit by name. -/
def Flags.read (f : Flags) (which : FlagBit) : Bool :=
  match which with
  | .carry    => f.carry
  | .zero     => f.zero
  | .sign     => f.sign
  | .overflow => f.overflow
  | .parity   => f.parity
  | .aux      => f.aux

/-- A machine state for the SSA fragment + memory + per-Ref Flags
    table. The full TSO-aware state lives in a weak-memory refinement
    (Fase 2 weeks 27-32). -/
structure MachineState where
  regs    : RegFile
  env     : Env
  mem     : Memory
  flags   : Ref → Flags    -- Flags Refs from WriteFlags map here
  deriving Inhabited

/-- The "empty" state: all zeroes. -/
def MachineState.empty : MachineState :=
  { regs  := fun _ => 0
  , env   := fun _ => 0
  , mem   := fun _ => 0
  , flags := fun _ => {} }

/-- Update the flags table at a specific Ref. -/
def MachineState.setFlags (s : MachineState) (r : Ref) (f : Flags) : MachineState :=
  { s with flags := fun r' => if r' = r then f else s.flags r' }

/-- F1-LN-006: step relation for the simple register / memory ops.
    Returns the new state; `none` if the op is outside this fragment.
    Control-flow ops (Jump, Return, ...) are handled by the function-
    level executor, not this per-stmt step. -/
def step (s : MachineState) : Stmt → Option MachineState
  | { result := some r, op := .constant v sz } =>
      some { s with env := s.env.extend r (maskToSize v sz) }
  | { result := some r, op := .loadReg g _ } =>
      some { s with env := s.env.extend r (s.regs g) }
  | { result := none,   op := .storeReg g v _ } =>
      some { s with regs := s.regs.set g (s.env v) }
  | { result := some r, op := .binop op lhs rhs sz } =>
      some { s with
             env := s.env.extend r (maskToSize (evalBinOp op (s.env lhs) (s.env rhs)) sz) }
  | { result := some r, op := .compare cc lhs rhs _ } =>
      some { s with env := s.env.extend r (evalCompare cc (s.env lhs) (s.env rhs)) }
  | { result := some r, op := .loadMem addrRef _ } =>
      some { s with env := s.env.extend r (s.mem.read64 (s.env addrRef).toNat) }
  | { result := none,   op := .storeMem addrRef value _ } =>
      some { s with mem := s.mem.write64 (s.env addrRef).toNat (s.env value) }
  | { result := some r, op := .writeFlags op lhs rhs _ } =>
      some (s.setFlags r (Flags.fromBinop op (s.env lhs) (s.env rhs)))
  | { result := some r, op := .readFlag flagsRef which } =>
      some { s with env := s.env.extend r
                       (if (s.flags flagsRef).read which then 1 else 0) }
  | _                                              => none

/-- F1-LN-007 — branch decision for CondJumpFlags. Returns the
    chosen successor block ID using the Flags Ref's bits and the
    requested CondCode. Caller looks up the resulting block id in
    `Function.blocks` and continues stepping there.

    Only the four CondCodes the C++ lowerer maps to ARM64 NZCV
    (`Eq`, `Ne`, `Slt`-via-Mi, `Cs`-via-Cc/Nc) are modelled here;
    the rest fall through to the false branch as a conservative
    placeholder until the full NZCV / Int64 reading lands.  -/
def stepCondJumpFlags (s : MachineState)
    (flagsRef : Ref) (code : CondCode)
    (ifTrue ifFalse : Nat) : Nat :=
  let f := s.flags flagsRef
  let taken : Bool :=
    match code with
    | CondCode.eq    => f.zero
    | CondCode.ne    => !f.zero
    | CondCode.cc    => f.carry
    | CondCode.nc    => !f.carry
    | CondCode.ov    => f.overflow
    | CondCode.noov  => !f.overflow
    | CondCode.mi    => f.sign
    | CondCode.pl    => !f.sign
    | _              => false
  if taken then ifTrue else ifFalse

end PrismaIR
