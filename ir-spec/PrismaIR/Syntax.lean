/-
  PrismaIR.Syntax — abstract syntax of the Prisma IR.

  Status: EARLY SKETCH (Fase 0, semana 1). The real IR has more opcodes,
  richer typing, and TSO annotations. This is just enough to:
    1. Prove that Lean 4 is a viable host for the formal spec.
    2. Let us write a first soundness proof of a trivial pass (DCE)
       against a toy semantics in Fase 1, semana 27-32.

  Design notes:
    * SSA form — every produced value has a unique `Ref`.
    * Refs are `Nat` for now; will become 32-bit offsets in the runtime
      data structure per the FEX compression idea (research note 2026-04-19).
    * OpSize is finite for now (i8/i16/i32/i64). SIMD is future work.
    * Memory ops come in TSO and non-TSO flavors: the TSO-adaptive pass
      (Pillar 3) rewrites TSO → non-TSO when proven safe. Correctness of
      that rewrite is the target of the Pillar 2 proof obligation.
-/

namespace PrismaIR

/-- The 16 general-purpose registers of x86_64. -/
inductive GPR where
  | rax | rcx | rdx | rbx | rsp | rbp | rsi | rdi
  | r8  | r9  | r10 | r11 | r12 | r13 | r14 | r15
  deriving DecidableEq, Repr, BEq, Hashable

/-- Operand sizes for integer values. SIMD sizes (128/256-bit) not modelled yet. -/
inductive OpSize where
  | i8 | i16 | i32 | i64
  deriving DecidableEq, Repr, BEq

/-- Width in bits of an OpSize. -/
def OpSize.widthBits : OpSize → Nat
  | .i8  => 8
  | .i16 => 16
  | .i32 => 32
  | .i64 => 64

/-- Unique identifier for an SSA value. Will be implemented as a 32-bit
    offset in the C++ runtime for memory compactness; `Nat` here for proof
    convenience. -/
abbrev Ref := Nat

/-- Pure binary arithmetic/logical ops. Flag-producing variants are separate
    ops to keep side-effects explicit (see `compare`). -/
inductive BinOp where
  | add | sub
  | and | or | xor
  | shl | shr | sar
  deriving DecidableEq, Repr, BEq

/-- x86 condition codes used by conditional branches and SETcc-like compare. -/
inductive CondCode where
  | eq  | ne
  | ult | ule | ugt | uge    -- unsigned
  | slt | sle | sgt | sge    -- signed
  deriving DecidableEq, Repr, BEq

/-- Core IR operation. An `Op` either produces exactly one value (pure ops,
    loads) or has side-effects (stores, control flow) and produces nothing.
    We model both with a single inductive for now; a future refinement will
    split `Op` into `ValOp` and `EffectOp` to match a more standard SSA. -/
inductive Op where
  -- Values out of thin air
  | constant (value : UInt64) (size : OpSize) : Op

  -- Context / register bank
  | loadReg  (reg : GPR) (size : OpSize)             : Op
  | storeReg (reg : GPR) (value : Ref) (size : OpSize) : Op

  -- Arithmetic / logic on two SSA refs
  | binop    (op : BinOp) (lhs rhs : Ref) (size : OpSize) : Op

  -- Comparison produces a 1-bit ref used by condJump / SETcc lowering
  | compare  (cc : CondCode) (lhs rhs : Ref) (size : OpSize) : Op

  -- Memory access
  | loadMem  (addr : Ref)              (size : OpSize) : Op
  | storeMem (addr value : Ref)        (size : OpSize) : Op

  -- Memory access with TSO semantics. The TSO-adaptive pass (Pillar 3) may
  -- rewrite these into the non-TSO variants above when the classifier proves
  -- the region is single-threaded or lock-free. Correctness: the rewrite
  -- must preserve sequential consistency of the containing program under
  -- the classifier's invariants.
  | loadMemTSO  (addr : Ref)           (size : OpSize) : Op
  | storeMemTSO (addr value : Ref)     (size : OpSize) : Op

  -- Control flow
  | jump     (target : Nat)                          : Op
  | condJump (cond : Ref) (ifTrue ifFalse : Nat)     : Op
  | ret                                              : Op

  deriving Repr

/-- One statement: either a pure op that binds its result to a Ref, or a
    side-effecting op with no result. We keep them uniform here for
    simplicity; a pass could separate them. -/
structure Stmt where
  /-- Ref bound by this statement, if any. `none` for stores / jumps / ret. -/
  result : Option Ref
  op : Op
  deriving Repr

/-- A basic block: a list of statements. The last one should be a control-flow
    op (`jump`, `condJump`, or `ret`). Well-formedness of this invariant is a
    future validator, not a type-level constraint in this early sketch. -/
structure BasicBlock where
  id : Nat
  stmts : List Stmt
  deriving Repr

/-- An IR function: a collection of basic blocks with a designated entry. -/
structure Function where
  blocks : List BasicBlock
  entry : Nat
  deriving Repr

end PrismaIR
