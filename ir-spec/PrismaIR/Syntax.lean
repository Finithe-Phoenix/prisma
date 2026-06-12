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
  | add | sub | mul
  | and | or | xor
  | shl | shr | sar
  | rol | ror
  | rcl | rcr
  -- F2-BK-007 — wide-form integer ops for x86 MUL/IMUL/DIV/IDIV
  -- writeback to rdx:rax / rax:rdx. uMulHi/sMulHi return the upper
  -- 64 bits of the unsigned/signed 64×64→128 product (i.e. rdx of
  -- the implicit rdx:rax pair). uDiv/sDiv return the quotient
  -- (rax of the implicit rax:rdx pair); uMod/sMod the remainder
  -- (rdx). See `evalBinOp` in `Semantics.lean` for the concrete
  -- semantics + the documented ARM64-on-zero-divisor divergence.
  | uMulHi | sMulHi
  | uDiv   | sDiv
  | uMod   | sMod
  deriving DecidableEq, Repr, BEq

/-- x86 condition codes used by conditional branches and SETcc-like compare.
    The flag-direct codes (cc/nc/ov/noov/mi/pl) mirror the C++ enum and let
    CondJumpRel test NZCV directly without going through a Compare. -/
inductive CondCode where
  | eq  | ne
  | ult | ule | ugt | uge    -- unsigned
  | slt | sle | sgt | sge    -- signed
  | cc  | nc                 -- carry clear / set
  | ov  | noov               -- overflow set / clear
  | mi  | pl                 -- sign set / clear
  deriving DecidableEq, Repr, BEq

/-- x86 segment registers. Only FS and GS carry meaningful base addresses
    on x86-64; CS/DS/ES/SS are conventionally zero-base. -/
inductive SegmentReg where
  | es | cs | ss | ds | fs | gs
  deriving DecidableEq, Repr, BEq

/-- Trap kinds for explicit guest exceptions (INT3, UD2, divide error). -/
inductive TrapKind where
  | sigtrap | sigill | sigfpe
  deriving DecidableEq, Repr, BEq

/-- Memory-fence kinds. Mfence = full barrier; Lfence = load-only;
    Sfence = store-only. Lowered to ARM64 `dmb ish / ishld / ishst`. -/
inductive FenceKind where
  | mfence | lfence | sfence
  deriving DecidableEq, Repr, BEq

/-- Floating-point precision (IEEE-754 binary32 / binary64). Half and
    quad precision are not modelled in the MVP. -/
inductive FpSize where
  | f32 | f64
  deriving DecidableEq, Repr, BEq

/-- Hot scalar floating-point binary operations: matches x86 SSE/AVX
    ADDSS / ADDSD / SUBSS / SUBSD / MULSS / MULSD / DIVSS / DIVSD. -/
inductive FpBinOp where
  | fadd | fsub | fmul | fdiv
  deriving DecidableEq, Repr, BEq

/-- Per-bit identifier in EFLAGS. Mirrors the C++ `FlagBit` enum. -/
inductive FlagBit where
  | carry | zero | sign | overflow | parity | aux
  deriving DecidableEq, Repr, BEq

/-- SHA-NI operation selector (F2-IR-060). Mirrors the C++
    `VecShaKind` enum. Lane conventions (per the Intel SDM): the SHA-1
    kinds keep W0/A in lane 3 (high dword); the SHA-256 kinds are
    lane-ascending (W0/A in lane 0). -/
inductive VecShaKind where
  | sha1Rnds4 | sha1Nexte | sha1Msg1 | sha1Msg2
  | sha256Rnds2 | sha256Msg1 | sha256Msg2
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

  -- Control flow (basic-block indexed, unused in the current MVP)
  | jump     (target : Nat)                          : Op
  | condJump (cond : Ref) (ifTrue ifFalse : Nat)     : Op
  | ret                                              : Op

  -- Indirect jump through an SSA Ref holding the target address.
  | jumpReg      (target : Ref)                               : Op

  -- Control flow (guest-PC indexed, used today). CmpFlags is a
  -- side-effecting op that sets an implicit flag bank; the next
  -- CondJumpRel reads that bank. No SSA result.
  | cmpFlags     (lhs rhs : Ref) (size : OpSize)              : Op
  | jumpRel      (targetGuestPc : UInt64)                     : Op
  | condJumpRel  (cc : CondCode)
                 (targetGuestPc fallthroughGuestPc : UInt64)  : Op

  -- Calls and returns (F1-IR-008).
  | callRel      (targetGuestPc returnGuestPc : UInt64)       : Op
  | callReg      (target : Ref) (returnGuestPc : UInt64)      : Op
  | retAdjusted  (popBytes : UInt64)                          : Op

  -- Architectural placeholders. Lowered today to halt-sentinel /
  -- zeroed-output stubs until the real semantics ships.
  | cpuid                                                     : Op
  | syscall                                                   : Op
  | trap         (kind : TrapKind)                            : Op

  -- Width adjustment.
  | extend       (value : Ref) (fromSize toSize : OpSize)
                 (signed : Bool)                              : Op
  | truncate     (value : Ref) (toSize : OpSize)              : Op

  -- Explicit memory fence.
  | fence        (kind : FenceKind)                           : Op

  -- Segment-base load (FS/GS for TLS). Pure, returns I64.
  | loadSegBase  (seg : SegmentReg)                           : Op

  -- Diagnostics.
  | guestPc      (pc : UInt64)                                : Op
  | inlineAsm    (bytes : List UInt8)                         : Op

  -- Floating-point (F1-IR-026 / F1-BK-013).
  | fpConstant   (bits : UInt64) (size : FpSize)              : Op
  | fpBinop      (op : FpBinOp) (lhs rhs : Ref) (size : FpSize) : Op

  -- Flags pillar (F1-IR-003 / F1-IR-004 / F1-IR-005 / F1-IR-007).
  | writeFlags   (op : BinOp) (lhs rhs : Ref) (size : OpSize) : Op
  | readFlag     (flags : Ref) (which : FlagBit)              : Op
  | condJumpFlags (flags : Ref) (cc : CondCode)
                  (ifTrue ifFalse : Nat)                      : Op

  -- F2-BK-008/009 — REP STOSB/MOVSB with bounded iteration. Block
  -- terminators; the lowering caps per-call iterations and re-enters
  -- via pcOfRep when RCX is non-zero post-loop. See
  -- `core/include/prisma/ir.hpp::kRepMaxBytesPerCall`. Semantics
  -- (iteration step relation) deferred to the MachineState-aware
  -- model; today the C++ runtime is the source of truth.
  | repStos      (size : OpSize) (reverse : Bool)
                 (pcOfRep pcAfterRep : UInt64)                : Op
  | repMovs      (size : OpSize) (reverse : Bool)
                 (pcOfRep pcAfterRep : UInt64)                : Op

  -- F2-IR-059 / F2-IR-060 — first SIMD-domain mirrors. The Lean model
  -- has no 128-bit value carrier yet (OpSize stops at i64, Env is
  -- Ref → UInt64), so these are constructor-only mirrors: operand Refs
  -- and descriptor fields are recorded, semantics deferred to the
  -- future vector-aware MachineState model; today the C++ runtime +
  -- the FIPS 180-4 full-digest e2e KATs are the source of truth.
  -- Mirroring now keeps the pass-soundness case-splits exhaustive
  -- while the vector story lands.
  --
  -- vecGather: per lane i (i < laneCount), with d = destLaneBase + i
  -- and x = indexLaneBase + i: if mask[d] has its MSB set, dest[d] =
  -- mem[base + (sext64(index[x]) <<< scaleShift)], else dest[d] =
  -- prev[d]. Masked-off lanes must not touch memory.
  | vecGather    (base index mask prev : Ref)
                 (scaleShift : UInt8) (elemIs64 indexIs64 : Bool)
                 (laneCount destLaneBase indexLaneBase : UInt8) : Op
  -- vecSha: `a` is the destination's prior value (RMW source 1), `b`
  -- the second operand, `wk` the implicit XMM0 pair (sha256Rnds2
  -- only; other kinds pass `b` again), `imm` the sha1Rnds4 round
  -- selector (imm8 & 3, zero for every other kind).
  | vecSha       (kind : VecShaKind) (a b wk : Ref)
                 (imm : UInt8)                                : Op

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
