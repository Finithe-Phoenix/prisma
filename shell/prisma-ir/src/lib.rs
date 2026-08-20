// Prisma IR types — SSA intermediate representation.
//
// Mirrors `core/include/prisma/ir.hpp` and `ir-spec/PrismaIR/Syntax.lean`.
// The Lean spec is authoritative; discrepancies are bugs.

#![deny(unsafe_op_in_unsafe_fn, unused_must_use)]
#![warn(clippy::all, clippy::pedantic, clippy::nursery)]
#![allow(clippy::derive_partial_eq_without_eq)]

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Basic enums
// ---------------------------------------------------------------------------

/// x86-64 general-purpose registers in architectural order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[repr(u8)]
pub enum Gpr {
    Rax = 0,
    Rcx,
    Rdx,
    Rbx,
    Rsp,
    Rbp,
    Rsi,
    Rdi,
    R8,
    R9,
    R10,
    R11,
    R12,
    R13,
    R14,
    R15,
}

pub const GPR_COUNT: usize = 16;

/// Operand size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[repr(u8)]
pub enum OpSize {
    I8 = 0,
    I16 = 1,
    I32 = 2,
    I64 = 3,
}

impl OpSize {
    #[must_use]
    pub const fn bit_width(self) -> u32 {
        match self {
            Self::I8 => 8,
            Self::I16 => 16,
            Self::I32 => 32,
            Self::I64 => 64,
        }
    }

    #[must_use]
    pub const fn mask(self) -> u64 {
        match self {
            Self::I8 => 0xFF,
            Self::I16 => 0xFFFF,
            Self::I32 => 0xFFFF_FFFF,
            Self::I64 => u64::MAX,
        }
    }
}

/// Binary operation kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[repr(u8)]
pub enum BinOpKind {
    Add = 0,
    Sub,
    Mul,
    And,
    Or,
    Xor,
    Shl,
    Shr,
    Sar,
    Rol,
    Ror,
    Rcl,
    Rcr,
    UMulHi,
    SMulHi,
    UDiv,
    SDiv,
    UMod,
    SMod,
    Pdep,
    Pext,
}

/// Which half of a full-width division result an IR statement materializes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[repr(u8)]
pub enum WideDivResult {
    Quotient = 0,
    Remainder,
}

/// Condition codes for `Compare`, `CondJumpRel`, `Select`, `CondJumpFlags`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[repr(u8)]
pub enum CondCode {
    Eq = 0,
    Ne,
    Ult,
    Ule,
    Ugt,
    Uge,
    Slt,
    Sle,
    Sgt,
    Sge,
    Cc,
    Nc,
    Ov,
    NoOv,
    Mi,
    Pl,
}

/// x86 segment registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[repr(u8)]
pub enum SegmentReg {
    Es = 0,
    Cs,
    Ss,
    Ds,
    Fs,
    Gs,
}

/// Flag bits for `ReadFlag`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[repr(u8)]
pub enum FlagBit {
    Carry = 0,
    Zero,
    Sign,
    Overflow,
    Parity,
    Aux,
}

/// How `StoreRflagsFromNzcv` derives the x86 CF bit from ARM64 NZCV.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[repr(u8)]
pub enum RflagsCarryMode {
    /// x86 CF is ARM64 C, used by add-like flag writers.
    ArmCarry = 0,
    /// x86 CF is NOT ARM64 C, used by subtract/compare borrow semantics.
    InvertArmCarry,
    /// x86 CF is cleared, used by logical flag writers.
    Clear,
    /// x86 CF is left unchanged while ZF/SF/OF are refreshed.
    Preserve,
}

/// Memory fence kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[repr(u8)]
pub enum FenceKind {
    Mfence = 0,
    Lfence,
    Sfence,
}

/// Trap kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[repr(u8)]
pub enum TrapKind {
    Sigtrap = 0,
    Sigill,
    Sigfpe,
}

/// SIMD lane widths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[repr(u8)]
pub enum VecLane {
    B16 = 0,
    H8,
    S4,
    D2,
}

/// SIMD integer binary operation kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[repr(u8)]
pub enum VecBinOpKind {
    Add = 0,
    Sub,
    And,
    Or,
    Xor,
    Mul,
    SqAdd,
    UqAdd,
    SqSub,
    UqSub,
    UMin,
    UMax,
    SMin,
    SMax,
    SMulHi,
    UMulHi,
    UMul32To64,
    SadBw,
    PairAddInt,
    PairSubInt,
}

/// SIMD FP comparison predicates (CMPPS/PD/SS/SD imm8 & 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[repr(u8)]
pub enum VecFpCmpPred {
    Eq = 0,
    Lt,
    Le,
    Unord,
    Neq,
    Nlt,
    Nle,
    Ord,
}

/// SIMD vector shift kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[repr(u8)]
pub enum VecShiftKind {
    ShiftL = 0,
    LogicalShr,
    ArithShr,
}

/// SIMD FP binary operation kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[repr(u8)]
pub enum VecFpBinOpKind {
    Add = 0,
    Sub,
    Mul,
    Div,
    Min,
    Max,
    Sqrt,
    HAdd,
}

/// SIMD FP size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[repr(u8)]
pub enum VecFpSize {
    S4 = 0,
    D2,
}

/// Packed F16C conversion direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[repr(u8)]
pub enum VecF16CvtKind {
    /// Convert four packed half-precision values from src[63:0] to four f32 lanes.
    PhToPs = 0,
    /// Convert four packed f32 lanes to four half-precision values in result[63:0].
    PsToPh,
}

/// Scalar FP size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[repr(u8)]
pub enum FpSize {
    F32 = 0,
    F64,
}

/// Scalar FP binary operation kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[repr(u8)]
pub enum FpBinOpKind {
    Add = 0,
    Sub,
    Mul,
    Div,
}

/// AES operation kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[repr(u8)]
pub enum VecAesKind {
    Enc = 0,
    EncLast,
    Dec,
    DecLast,
    Imc,
}

/// SHA operation kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[repr(u8)]
pub enum VecShaKind {
    Sha1Rnds4 = 0,
    Sha1Nexte,
    Sha1Msg1,
    Sha1Msg2,
    Sha256Rnds2,
    Sha256Msg1,
    Sha256Msg2,
}

// ---------------------------------------------------------------------------
// Ref — SSA value identifier
// ---------------------------------------------------------------------------

/// SSA reference: offset into the containing function's statement list.
pub type Ref = u32;

pub const INVALID_REF: Ref = u32::MAX;

// ---------------------------------------------------------------------------
// Op variants
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Constant {
    pub value: u64,
    pub size: OpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadReg {
    pub reg: Gpr,
    pub size: OpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoreReg {
    pub reg: Gpr,
    pub value: Ref,
    pub size: OpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadSegBase {
    pub seg: SegmentReg,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BinOp {
    pub op: BinOpKind,
    pub lhs: Ref,
    pub rhs: Ref,
    pub size: OpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WideDiv {
    /// High half of the dividend (`RDX` for x86-64 `DIV/IDIV r/m64`).
    pub high: Ref,
    /// Low half of the dividend (`RAX` for x86-64 `DIV/IDIV r/m64`).
    pub low: Ref,
    pub divisor: Ref,
    pub signed: bool,
    pub result: WideDivResult,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Compare {
    pub cc: CondCode,
    pub lhs: Ref,
    pub rhs: Ref,
    pub size: OpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Select {
    pub cc: CondCode,
    pub true_value: Ref,
    pub false_value: Ref,
    pub size: OpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadMem {
    pub addr: Ref,
    pub size: OpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoreMem {
    pub addr: Ref,
    pub value: Ref,
    pub size: OpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadMemTSO {
    pub addr: Ref,
    pub size: OpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoreMemTSO {
    pub addr: Ref,
    pub value: Ref,
    pub size: OpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AtomicCmpxchg {
    pub addr: Ref,
    pub expected: Ref,
    pub new_value: Ref,
    pub size: OpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AtomicXchg {
    pub addr: Ref,
    pub value: Ref,
    pub size: OpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AtomicXadd {
    pub addr: Ref,
    pub value: Ref,
    pub size: OpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AtomicCmpxchgPair {
    pub addr: Ref,
    pub expected_low: Ref,
    pub expected_high: Ref,
    pub new_low: Ref,
    pub new_high: Ref,
    pub old_high: Ref,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Jump {
    pub target_block: u32,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CondJump {
    pub cond: Ref,
    pub if_true: u32,
    pub if_false: u32,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Return;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CmpFlags {
    pub lhs: Ref,
    pub rhs: Ref,
    pub size: OpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AluFlags {
    pub op: BinOpKind,
    pub lhs: Ref,
    pub rhs: Ref,
    pub size: OpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AluFlagsPreserveCarry {
    pub op: BinOpKind,
    pub lhs: Ref,
    pub rhs: Ref,
    pub size: OpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JumpReg {
    pub target: Ref,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JumpRel {
    pub target_guest_pc: u64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CondJumpRel {
    pub cc: CondCode,
    pub target_guest_pc: u64,
    pub fallthrough_guest_pc: u64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallRel {
    pub target_guest_pc: u64,
    pub return_guest_pc: u64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallReg {
    pub target: Ref,
    pub return_guest_pc: u64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetAdjusted {
    pub pop_bytes: u64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cpuid;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Xgetbv;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rdtsc;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Syscall;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Extend {
    pub value: Ref,
    pub from_size: OpSize,
    pub to_size: OpSize,
    pub is_signed: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Truncate {
    pub value: Ref,
    pub to_size: OpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fence {
    pub kind: FenceKind,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GuestPc {
    pub pc: u64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WriteFlags {
    pub op: BinOpKind,
    pub lhs: Ref,
    pub rhs: Ref,
    pub size: OpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadFlag {
    pub flags: Ref,
    pub which: FlagBit,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CondJumpFlags {
    pub flags: Ref,
    pub cc: CondCode,
    pub if_true: u32,
    pub if_false: u32,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RspAdjust {
    pub delta_bytes: i64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecConstant {
    pub lo: u64,
    pub hi: u64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecBinOp {
    pub op: VecBinOpKind,
    pub lhs: Ref,
    pub rhs: Ref,
    pub lane: VecLane,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecClMul {
    pub lhs: Ref,
    pub rhs: Ref,
    pub lhs_high: bool,
    pub rhs_high: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecF16Cvt {
    pub kind: VecF16CvtKind,
    pub src: Ref,
    /// VCVTPS2PH rounding immediate. Ignored for VCVTPH2PS.
    pub rounding: u8,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadVecReg {
    pub xmm_index: u8,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoreVecReg {
    pub xmm_index: u8,
    pub value: Ref,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadVec {
    pub addr: Ref,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoreVec {
    pub addr: Ref,
    pub value: Ref,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecFpBinOp {
    pub op: VecFpBinOpKind,
    pub lhs: Ref,
    pub rhs: Ref,
    pub size: VecFpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecFpScalarBinOp {
    pub op: VecFpBinOpKind,
    pub lhs: Ref,
    pub rhs: Ref,
    pub size: FpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct XmmFromGpr {
    pub value: Ref,
    pub size: OpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GprFromXmm {
    pub value: Ref,
    pub size: OpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecCmp {
    pub kind: VecCmpKind,
    pub lhs: Ref,
    pub rhs: Ref,
    pub lane: VecLane,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PcmpStrIndex {
    pub lhs: Ref,
    pub rhs: Ref,
    pub lhs_len: Option<Ref>,
    pub rhs_len: Option<Ref>,
    pub imm8: u8,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PcmpStrMask {
    pub lhs: Ref,
    pub rhs: Ref,
    pub lhs_len: Option<Ref>,
    pub rhs_len: Option<Ref>,
    pub imm8: u8,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PcmpStrFlags {
    pub lhs: Ref,
    pub rhs: Ref,
    pub lhs_len: Option<Ref>,
    pub rhs_len: Option<Ref>,
    pub imm8: u8,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecShuffle32x4 {
    pub src: Ref,
    pub control: u8,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecUnpack {
    pub is_high: bool,
    pub lhs: Ref,
    pub rhs: Ref,
    pub lane: VecLane,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecShiftImm {
    pub kind: VecShiftKind,
    pub src: Ref,
    pub count: u8,
    pub lane: VecLane,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecShiftBytes {
    pub is_left: bool,
    pub src: Ref,
    pub count: u8,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntToFpScalar {
    pub value: Ref,
    pub int_size: OpSize,
    pub fp_size: FpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FpToIntScalar {
    pub value: Ref,
    pub fp_size: FpSize,
    pub int_size: OpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FpCvtScalar {
    pub lhs: Ref,
    pub src: Ref,
    pub src_size: FpSize,
    pub dst_size: FpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecShuffle2Src {
    pub is_pd: bool,
    pub lhs: Ref,
    pub rhs: Ref,
    pub control: u8,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecInsertLane {
    pub lhs_xmm: Ref,
    pub value: Ref,
    pub lane_idx: u8,
    pub lane: VecLane,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecExtractLaneU {
    pub src_xmm: Ref,
    pub lane_idx: u8,
    pub lane: VecLane,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecMaskMsb {
    pub src_xmm: Ref,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WriteFlagsFp {
    pub lhs: Ref,
    pub rhs: Ref,
    pub size: FpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecShuffleH4 {
    pub is_high: bool,
    pub src: Ref,
    pub control: u8,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecMaskFp {
    pub src_xmm: Ref,
    pub is_pd: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecFpCompare {
    pub lhs: Ref,
    pub rhs: Ref,
    pub size: FpSize,
    pub pred: VecFpCmpPred,
    pub is_packed: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecPshufb {
    pub src: Ref,
    pub mask: Ref,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecAbs {
    pub src: Ref,
    pub lane: VecLane,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecAlignr {
    pub lhs: Ref,
    pub rhs: Ref,
    pub count: u8,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecExtend {
    pub src: Ref,
    pub narrow_lane: VecLane,
    pub wide_lane: VecLane,
    pub is_signed: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecFpRound {
    pub lhs: Ref,
    pub src: Ref,
    pub size: FpSize,
    pub mode: u8,
    pub is_packed: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Popcnt {
    pub value: Ref,
    pub size: OpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lzcnt {
    pub value: Ref,
    pub size: OpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tzcnt {
    pub value: Ref,
    pub size: OpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WriteFlagsPopcnt {
    pub src: Ref,
    pub size: OpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WriteFlagsCountZero {
    pub src: Ref,
    pub result: Ref,
    pub size: OpSize,
}
/// Loads the persistent x86 carry flag (0 or 1) into `result`.
///
/// The host NZCV carry is transient and SSA-scoped; multi-precision ADC/SBB
/// need CF to survive between instructions, so it lives in a dedicated
/// `CpuStateFrame` slot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadCarry;
/// Materialises the carry-out of a preceding flag-setting op into `result` (0/1).
///
/// `from_sub` inverts: ARM64 sets C = NOT(borrow) on subtraction, so x86 CF
/// (borrow) is `cset cc` after a sub and `cset cs` after an add.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadCarryOut {
    pub flags: Ref,
    pub from_sub: bool,
}
/// Stores `value` (must be 0 or 1) into the persistent CF slot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoreCarry {
    pub value: Ref,
}
/// Loads the persistent x86 RFLAGS subset into `result`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadRflags;
/// Stores the persistent x86 RFLAGS subset from `value`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoreRflags {
    pub value: Ref,
}
/// Publishes the transient ARM64 NZCV flags into the persistent x86 RFLAGS
/// subset. ZF/SF/OF always come from NZCV; CF follows `carry`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoreRflagsFromNzcv {
    pub carry: RflagsCarryMode,
    pub pf: Option<Ref>,
    pub af: Option<Ref>,
}
/// Publishes explicit 0/1 refs into the persistent x86 RFLAGS subset while
/// preserving CF. PF/AF are optional so producers can avoid inventing undefined
/// or not-yet-modelled bits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoreRflagsFromBits {
    pub pf: Option<Ref>,
    pub af: Option<Ref>,
    pub zf: Ref,
    pub sf: Ref,
    pub of: Ref,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecBlend {
    pub dst: Ref,
    pub src: Ref,
    pub mask: Ref,
    pub lane: VecLane,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WriteFlagsPtest {
    pub lhs: Ref,
    pub rhs: Ref,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WriteFlagsPtestYmm {
    pub lo_lhs: Ref,
    pub lo_rhs: Ref,
    pub hi_lhs: Ref,
    pub hi_rhs: Ref,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecTbl2 {
    pub src_lo: Ref,
    pub src_hi: Ref,
    pub idx: Ref,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecAes {
    pub src: Ref,
    pub key: Ref,
    pub kind: VecAesKind,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecAesKeygenAssist {
    pub src: Ref,
    pub rcon: u8,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecSha {
    pub kind: VecShaKind,
    pub a: Ref,
    pub b: Ref,
    pub wk: Ref,
    pub imm: u8,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bswap {
    pub value: Ref,
    pub size: OpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Crc32c {
    pub crc: Ref,
    pub data: Ref,
    pub data_size: OpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecGather {
    pub base: Ref,
    pub index: Ref,
    pub mask: Ref,
    pub prev: Ref,
    pub scale_shift: u8,
    pub elem_is64: u8,
    pub index_is64: u8,
    pub lane_count: u8,
    pub dest_lane_base: u8,
    pub index_lane_base: u8,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadVecRegHi {
    pub ymm_index: u8,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoreVecRegHi {
    pub ymm_index: u8,
    pub value: Ref,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecFpFma {
    pub a: Ref,
    pub b: Ref,
    pub c: Ref,
    pub neg_addend: bool,
    pub neg_mul: bool,
    pub size: VecFpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecFpScalarFma {
    pub a: Ref,
    pub b: Ref,
    pub c: Ref,
    pub scalar_upper: Ref,
    pub neg_addend: bool,
    pub neg_mul: bool,
    pub size: FpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepStos {
    pub size: OpSize,
    pub reverse: bool,
    pub pc_of_rep: u64,
    pub pc_after_rep: u64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepMovs {
    pub size: OpSize,
    pub reverse: bool,
    pub pc_of_rep: u64,
    pub pc_after_rep: u64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct X87Load {
    pub st_index: u8,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct X87Store {
    pub st_index: u8,
    pub value: Ref,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct X87Push {
    pub value: Ref,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct X87Pop;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InlineAsm {
    pub bytes: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trap {
    pub kind: TrapKind,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrapIf {
    pub condition: Ref,
    pub kind: TrapKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FpConstant {
    pub bits: u64,
    pub size: FpSize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FpBinOp {
    pub op: FpBinOpKind,
    pub lhs: Ref,
    pub rhs: Ref,
    pub size: FpSize,
}

/// Comparison kinds for `VecCmp`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[repr(u8)]
pub enum VecCmpKind {
    Eq = 0,
    Gt,
}

// ---------------------------------------------------------------------------
// Op — the core IR opcode enum
// ---------------------------------------------------------------------------

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Op {
    Constant(Constant),
    LoadReg(LoadReg),
    StoreReg(StoreReg),
    LoadSegBase(LoadSegBase),
    BinOp(BinOp),
    WideDiv(WideDiv),
    Compare(Compare),
    Select(Select),
    LoadMem(LoadMem),
    StoreMem(StoreMem),
    LoadMemTSO(LoadMemTSO),
    StoreMemTSO(StoreMemTSO),
    AtomicCmpxchg(AtomicCmpxchg),
    AtomicXchg(AtomicXchg),
    AtomicXadd(AtomicXadd),
    AtomicCmpxchgPair(AtomicCmpxchgPair),
    Jump(Jump),
    CondJump(CondJump),
    Return(Return),
    CmpFlags(CmpFlags),
    AluFlags(AluFlags),
    AluFlagsPreserveCarry(AluFlagsPreserveCarry),
    JumpReg(JumpReg),
    JumpRel(JumpRel),
    CondJumpRel(CondJumpRel),
    CallRel(CallRel),
    CallReg(CallReg),
    RetAdjusted(RetAdjusted),
    Cpuid(Cpuid),
    Xgetbv(Xgetbv),
    Rdtsc(Rdtsc),
    Syscall(Syscall),
    Trap(Trap),
    TrapIf(TrapIf),
    Extend(Extend),
    Truncate(Truncate),
    Fence(Fence),
    GuestPc(GuestPc),
    InlineAsm(InlineAsm),
    FpConstant(FpConstant),
    FpBinOp(FpBinOp),
    WriteFlags(WriteFlags),
    ReadFlag(ReadFlag),
    CondJumpFlags(CondJumpFlags),
    RspAdjust(RspAdjust),
    VecConstant(VecConstant),
    VecBinOp(VecBinOp),
    VecClMul(VecClMul),
    VecF16Cvt(VecF16Cvt),
    LoadVecReg(LoadVecReg),
    StoreVecReg(StoreVecReg),
    LoadVec(LoadVec),
    StoreVec(StoreVec),
    VecFpBinOp(VecFpBinOp),
    VecFpScalarBinOp(VecFpScalarBinOp),
    XmmFromGpr(XmmFromGpr),
    GprFromXmm(GprFromXmm),
    VecCmp(VecCmp),
    PcmpStrIndex(PcmpStrIndex),
    PcmpStrMask(PcmpStrMask),
    PcmpStrFlags(PcmpStrFlags),
    VecShuffle32x4(VecShuffle32x4),
    VecUnpack(VecUnpack),
    VecShiftImm(VecShiftImm),
    VecShiftBytes(VecShiftBytes),
    IntToFpScalar(IntToFpScalar),
    FpToIntScalar(FpToIntScalar),
    FpCvtScalar(FpCvtScalar),
    VecShuffle2Src(VecShuffle2Src),
    VecInsertLane(VecInsertLane),
    VecExtractLaneU(VecExtractLaneU),
    VecMaskMsb(VecMaskMsb),
    WriteFlagsFp(WriteFlagsFp),
    VecShuffleH4(VecShuffleH4),
    VecMaskFp(VecMaskFp),
    VecFpCompare(VecFpCompare),
    VecPshufb(VecPshufb),
    VecAbs(VecAbs),
    VecAlignr(VecAlignr),
    VecExtend(VecExtend),
    VecFpRound(VecFpRound),
    Popcnt(Popcnt),
    Lzcnt(Lzcnt),
    Tzcnt(Tzcnt),
    WriteFlagsPopcnt(WriteFlagsPopcnt),
    WriteFlagsCountZero(WriteFlagsCountZero),
    VecBlend(VecBlend),
    WriteFlagsPtest(WriteFlagsPtest),
    WriteFlagsPtestYmm(WriteFlagsPtestYmm),
    VecTbl2(VecTbl2),
    VecAes(VecAes),
    VecAesKeygenAssist(VecAesKeygenAssist),
    VecSha(VecSha),
    Bswap(Bswap),
    Crc32c(Crc32c),
    VecGather(VecGather),
    LoadVecRegHi(LoadVecRegHi),
    StoreVecRegHi(StoreVecRegHi),
    VecFpFma(VecFpFma),
    VecFpScalarFma(VecFpScalarFma),
    RepStos(RepStos),
    RepMovs(RepMovs),
    X87Load(X87Load),
    X87Store(X87Store),
    X87Push(X87Push),
    X87Pop(X87Pop),
    LoadCarry(LoadCarry),
    ReadCarryOut(ReadCarryOut),
    StoreCarry(StoreCarry),
    LoadRflags(LoadRflags),
    StoreRflags(StoreRflags),
    StoreRflagsFromNzcv(StoreRflagsFromNzcv),
    StoreRflagsFromBits(StoreRflagsFromBits),
}

// ---------------------------------------------------------------------------
// Stmt, BasicBlock, Function
// ---------------------------------------------------------------------------

/// A single IR statement. `result` is `None` for side-effect-only ops
/// (`StoreReg`, `StoreMem`, `Jump`, `Return`, etc.).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stmt {
    pub result: Option<Ref>,
    pub op: Op,
}

impl Stmt {
    #[must_use]
    pub const fn new(result: Option<Ref>, op: Op) -> Self {
        Self { result, op }
    }

    /// Return every SSA ref defined by this statement.
    ///
    /// Most operations define at most `result`; pair compare-exchange also
    /// defines the observed high half stored in `old_high`.
    pub fn defined_refs(&self) -> impl Iterator<Item = Ref> {
        let secondary = match &self.op {
            Op::AtomicCmpxchgPair(pair) => Some(pair.old_high),
            _ => None,
        };
        self.result.into_iter().chain(secondary)
    }

    /// Apply `f` to every SSA value reference in this statement — its result
    /// (if any) and every operand ref inside the op. Block ids, guest
    /// addresses, register indices, and immediates are left untouched.
    pub fn map_refs(&mut self, mut f: impl FnMut(Ref) -> Ref) {
        if let Some(r) = self.result {
            self.result = Some(f(r));
        }
        self.op.map_refs(f);
    }
}

impl Op {
    /// Apply `f` to every SSA value reference stored in this op, including
    /// operands and secondary results. Used to renumber SSA refs when
    /// concatenating independently-decoded instructions into one basic block.
    /// The match is exhaustive so a new `Op` variant forces this to be updated.
    #[allow(clippy::too_many_lines)]
    pub fn map_refs(&mut self, mut f: impl FnMut(Ref) -> Ref) {
        match self {
            Self::StoreReg(x) => x.value = f(x.value),
            Self::BinOp(x) => {
                x.lhs = f(x.lhs);
                x.rhs = f(x.rhs);
            }
            Self::WideDiv(x) => {
                x.high = f(x.high);
                x.low = f(x.low);
                x.divisor = f(x.divisor);
            }
            Self::Compare(x) => {
                x.lhs = f(x.lhs);
                x.rhs = f(x.rhs);
            }
            Self::Select(x) => {
                x.true_value = f(x.true_value);
                x.false_value = f(x.false_value);
            }
            Self::LoadMem(x) => x.addr = f(x.addr),
            Self::StoreMem(x) => {
                x.addr = f(x.addr);
                x.value = f(x.value);
            }
            Self::LoadMemTSO(x) => x.addr = f(x.addr),
            Self::StoreMemTSO(x) => {
                x.addr = f(x.addr);
                x.value = f(x.value);
            }
            Self::AtomicCmpxchg(x) => {
                x.addr = f(x.addr);
                x.expected = f(x.expected);
                x.new_value = f(x.new_value);
            }
            Self::AtomicXchg(x) => {
                x.addr = f(x.addr);
                x.value = f(x.value);
            }
            Self::AtomicXadd(x) => {
                x.addr = f(x.addr);
                x.value = f(x.value);
            }
            Self::AtomicCmpxchgPair(x) => {
                x.addr = f(x.addr);
                x.expected_low = f(x.expected_low);
                x.expected_high = f(x.expected_high);
                x.new_low = f(x.new_low);
                x.new_high = f(x.new_high);
                x.old_high = f(x.old_high);
            }
            Self::CondJump(x) => x.cond = f(x.cond),
            Self::CmpFlags(x) => {
                x.lhs = f(x.lhs);
                x.rhs = f(x.rhs);
            }
            Self::AluFlags(x) => {
                x.lhs = f(x.lhs);
                x.rhs = f(x.rhs);
            }
            Self::AluFlagsPreserveCarry(x) => {
                x.lhs = f(x.lhs);
                x.rhs = f(x.rhs);
            }
            Self::JumpReg(x) => x.target = f(x.target),
            Self::CallReg(x) => x.target = f(x.target),
            Self::Extend(x) => x.value = f(x.value),
            Self::Truncate(x) => x.value = f(x.value),
            Self::WriteFlags(x) => {
                x.lhs = f(x.lhs);
                x.rhs = f(x.rhs);
            }
            Self::ReadFlag(x) => x.flags = f(x.flags),
            Self::CondJumpFlags(x) => x.flags = f(x.flags),
            Self::FpBinOp(x) => {
                x.lhs = f(x.lhs);
                x.rhs = f(x.rhs);
            }
            Self::VecBinOp(x) => {
                x.lhs = f(x.lhs);
                x.rhs = f(x.rhs);
            }
            Self::VecClMul(x) => {
                x.lhs = f(x.lhs);
                x.rhs = f(x.rhs);
            }
            Self::VecF16Cvt(x) => x.src = f(x.src),
            Self::StoreVecReg(x) => x.value = f(x.value),
            Self::LoadVec(x) => x.addr = f(x.addr),
            Self::StoreVec(x) => {
                x.addr = f(x.addr);
                x.value = f(x.value);
            }
            Self::VecFpBinOp(x) => {
                x.lhs = f(x.lhs);
                x.rhs = f(x.rhs);
            }
            Self::VecFpScalarBinOp(x) => {
                x.lhs = f(x.lhs);
                x.rhs = f(x.rhs);
            }
            Self::XmmFromGpr(x) => x.value = f(x.value),
            Self::GprFromXmm(x) => x.value = f(x.value),
            Self::VecCmp(x) => {
                x.lhs = f(x.lhs);
                x.rhs = f(x.rhs);
            }
            Self::PcmpStrIndex(x) => {
                x.lhs = f(x.lhs);
                x.rhs = f(x.rhs);
                if let Some(r) = x.lhs_len {
                    x.lhs_len = Some(f(r));
                }
                if let Some(r) = x.rhs_len {
                    x.rhs_len = Some(f(r));
                }
            }
            Self::PcmpStrMask(x) => {
                x.lhs = f(x.lhs);
                x.rhs = f(x.rhs);
                if let Some(r) = x.lhs_len {
                    x.lhs_len = Some(f(r));
                }
                if let Some(r) = x.rhs_len {
                    x.rhs_len = Some(f(r));
                }
            }
            Self::PcmpStrFlags(x) => {
                x.lhs = f(x.lhs);
                x.rhs = f(x.rhs);
                if let Some(r) = x.lhs_len {
                    x.lhs_len = Some(f(r));
                }
                if let Some(r) = x.rhs_len {
                    x.rhs_len = Some(f(r));
                }
            }
            Self::VecShuffle32x4(x) => x.src = f(x.src),
            Self::VecUnpack(x) => {
                x.lhs = f(x.lhs);
                x.rhs = f(x.rhs);
            }
            Self::VecShiftImm(x) => x.src = f(x.src),
            Self::VecShiftBytes(x) => x.src = f(x.src),
            Self::IntToFpScalar(x) => x.value = f(x.value),
            Self::FpToIntScalar(x) => x.value = f(x.value),
            Self::FpCvtScalar(x) => {
                x.lhs = f(x.lhs);
                x.src = f(x.src);
            }
            Self::VecShuffle2Src(x) => {
                x.lhs = f(x.lhs);
                x.rhs = f(x.rhs);
            }
            Self::VecInsertLane(x) => {
                x.lhs_xmm = f(x.lhs_xmm);
                x.value = f(x.value);
            }
            Self::VecExtractLaneU(x) => x.src_xmm = f(x.src_xmm),
            Self::VecMaskMsb(x) => x.src_xmm = f(x.src_xmm),
            Self::WriteFlagsFp(x) => {
                x.lhs = f(x.lhs);
                x.rhs = f(x.rhs);
            }
            Self::VecShuffleH4(x) => x.src = f(x.src),
            Self::VecMaskFp(x) => x.src_xmm = f(x.src_xmm),
            Self::VecFpCompare(x) => {
                x.lhs = f(x.lhs);
                x.rhs = f(x.rhs);
            }
            Self::VecPshufb(x) => {
                x.src = f(x.src);
                x.mask = f(x.mask);
            }
            Self::VecAbs(x) => x.src = f(x.src),
            Self::VecAlignr(x) => {
                x.lhs = f(x.lhs);
                x.rhs = f(x.rhs);
            }
            Self::VecExtend(x) => x.src = f(x.src),
            Self::VecFpRound(x) => {
                x.lhs = f(x.lhs);
                x.src = f(x.src);
            }
            Self::Popcnt(x) => x.value = f(x.value),
            Self::Lzcnt(x) => x.value = f(x.value),
            Self::Tzcnt(x) => x.value = f(x.value),
            Self::WriteFlagsPopcnt(x) => x.src = f(x.src),
            Self::WriteFlagsCountZero(x) => {
                x.src = f(x.src);
                x.result = f(x.result);
            }
            Self::VecBlend(x) => {
                x.dst = f(x.dst);
                x.src = f(x.src);
                x.mask = f(x.mask);
            }
            Self::WriteFlagsPtest(x) => {
                x.lhs = f(x.lhs);
                x.rhs = f(x.rhs);
            }
            Self::WriteFlagsPtestYmm(x) => {
                x.lo_lhs = f(x.lo_lhs);
                x.lo_rhs = f(x.lo_rhs);
                x.hi_lhs = f(x.hi_lhs);
                x.hi_rhs = f(x.hi_rhs);
            }
            Self::VecTbl2(x) => {
                x.src_lo = f(x.src_lo);
                x.src_hi = f(x.src_hi);
                x.idx = f(x.idx);
            }
            Self::VecAes(x) => {
                x.src = f(x.src);
                x.key = f(x.key);
            }
            Self::VecAesKeygenAssist(x) => x.src = f(x.src),
            Self::VecSha(x) => {
                x.a = f(x.a);
                x.b = f(x.b);
                x.wk = f(x.wk);
            }
            Self::Bswap(x) => x.value = f(x.value),
            Self::Crc32c(x) => {
                x.crc = f(x.crc);
                x.data = f(x.data);
            }
            Self::VecGather(x) => {
                x.base = f(x.base);
                x.index = f(x.index);
                x.mask = f(x.mask);
                x.prev = f(x.prev);
            }
            Self::StoreVecRegHi(x) => x.value = f(x.value),
            Self::VecFpFma(x) => {
                x.a = f(x.a);
                x.b = f(x.b);
                x.c = f(x.c);
            }
            Self::VecFpScalarFma(x) => {
                x.a = f(x.a);
                x.b = f(x.b);
                x.c = f(x.c);
                x.scalar_upper = f(x.scalar_upper);
            }
            Self::X87Store(x) => x.value = f(x.value),
            Self::X87Push(x) => x.value = f(x.value),
            Self::ReadCarryOut(x) => x.flags = f(x.flags),
            Self::StoreCarry(x) => x.value = f(x.value),
            Self::StoreRflags(x) => x.value = f(x.value),
            Self::TrapIf(x) => x.condition = f(x.condition),
            // Ops with no SSA operand refs: constants, register/segment loads,
            // block/guest-address control transfers, fences, CPU queries,
            // string ops, and the x87 load/pop forms.
            Self::Constant(_)
            | Self::LoadReg(_)
            | Self::LoadSegBase(_)
            | Self::Jump(_)
            | Self::Return(_)
            | Self::JumpRel(_)
            | Self::CondJumpRel(_)
            | Self::CallRel(_)
            | Self::RetAdjusted(_)
            | Self::Cpuid(_)
            | Self::Xgetbv(_)
            | Self::Rdtsc(_)
            | Self::Syscall(_)
            | Self::Trap(_)
            | Self::Fence(_)
            | Self::GuestPc(_)
            | Self::InlineAsm(_)
            | Self::FpConstant(_)
            | Self::RspAdjust(_)
            | Self::VecConstant(_)
            | Self::LoadVecReg(_)
            | Self::LoadVecRegHi(_)
            | Self::RepStos(_)
            | Self::RepMovs(_)
            | Self::X87Load(_)
            | Self::X87Pop(_)
            | Self::LoadCarry(_)
            | Self::LoadRflags(_) => {}
            Self::StoreRflagsFromNzcv(x) => {
                if let Some(pf) = x.pf {
                    x.pf = Some(f(pf));
                }
                if let Some(af) = x.af {
                    x.af = Some(f(af));
                }
            }
            Self::StoreRflagsFromBits(x) => {
                if let Some(pf) = x.pf {
                    x.pf = Some(f(pf));
                }
                if let Some(af) = x.af {
                    x.af = Some(f(af));
                }
                x.zf = f(x.zf);
                x.sf = f(x.sf);
                x.of = f(x.of);
            }
        }
    }
}

/// A basic block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BasicBlock {
    pub id: u32,
    pub stmts: Vec<Stmt>,
}

/// An IR function (multiple basic blocks).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Function {
    pub blocks: Vec<BasicBlock>,
    pub entry: u32,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Mask a 64-bit value to the width of `size`.
#[must_use]
pub const fn mask_to_size(v: u64, size: OpSize) -> u64 {
    v & size.mask()
}

/// Number of XMM registers.
pub const XMM_COUNT: usize = 16;

/// Max bytes a single REP STOS/MOVS invocation may write (16 MiB).
pub const REP_MAX_BYTES_PER_CALL: u64 = 16 << 20;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_refs_shifts_operands_and_result() {
        // r2 = r0 + r1  ->  shift by 10  ->  r12 = r10 + r11
        let mut stmt = Stmt::new(
            Some(2),
            Op::BinOp(BinOp {
                op: BinOpKind::Add,
                lhs: 0,
                rhs: 1,
                size: OpSize::I64,
            }),
        );
        stmt.map_refs(|r| r + 10);
        assert_eq!(stmt.result, Some(12));
        match stmt.op {
            Op::BinOp(b) => {
                assert_eq!((b.lhs, b.rhs), (10, 11));
            }
            _ => panic!("op changed"),
        }
    }

    #[test]
    fn map_refs_shifts_atomic_pair_secondary_result() {
        let mut stmt = Stmt::new(
            Some(6),
            Op::AtomicCmpxchgPair(AtomicCmpxchgPair {
                addr: 0,
                expected_low: 1,
                expected_high: 2,
                new_low: 3,
                new_high: 4,
                old_high: 7,
            }),
        );

        stmt.map_refs(|r| r + 10);

        assert_eq!(stmt.result, Some(16));
        match stmt.op {
            Op::AtomicCmpxchgPair(cas) => {
                assert_eq!(cas.addr, 10);
                assert_eq!(cas.expected_low, 11);
                assert_eq!(cas.expected_high, 12);
                assert_eq!(cas.new_low, 13);
                assert_eq!(cas.new_high, 14);
                assert_eq!(cas.old_high, 17);
            }
            _ => panic!("op changed"),
        }
    }

    #[test]
    fn defined_refs_include_atomic_pair_secondary_result() {
        let stmt = Stmt::new(
            Some(6),
            Op::AtomicCmpxchgPair(AtomicCmpxchgPair {
                addr: 0,
                expected_low: 1,
                expected_high: 2,
                new_low: 3,
                new_high: 4,
                old_high: 7,
            }),
        );

        assert_eq!(stmt.defined_refs().collect::<Vec<_>>(), vec![6, 7]);
        assert_eq!(
            Stmt::new(None, Op::Return(Return))
                .defined_refs()
                .collect::<Vec<_>>(),
            Vec::<Ref>::new()
        );
    }

    #[test]
    fn map_refs_shifts_vec_clmul_operands() {
        let mut stmt = Stmt::new(
            Some(2),
            Op::VecClMul(VecClMul {
                lhs: 0,
                rhs: 1,
                lhs_high: true,
                rhs_high: false,
            }),
        );

        stmt.map_refs(|r| r + 10);

        assert_eq!(stmt.result, Some(12));
        match stmt.op {
            Op::VecClMul(p) => {
                assert_eq!(p.lhs, 10);
                assert_eq!(p.rhs, 11);
                assert!(p.lhs_high);
                assert!(!p.rhs_high);
            }
            _ => panic!("op changed"),
        }
    }

    #[test]
    fn map_refs_shifts_vec_f16cvt_operand() {
        let mut stmt = Stmt::new(
            Some(2),
            Op::VecF16Cvt(VecF16Cvt {
                kind: VecF16CvtKind::PsToPh,
                src: 1,
                rounding: 0,
            }),
        );

        stmt.map_refs(|r| r + 10);

        assert_eq!(stmt.result, Some(12));
        match stmt.op {
            Op::VecF16Cvt(p) => {
                assert_eq!(p.kind, VecF16CvtKind::PsToPh);
                assert_eq!(p.src, 11);
                assert_eq!(p.rounding, 0);
            }
            _ => panic!("op changed"),
        }
    }

    #[test]
    fn map_refs_shifts_pcmp_string_lengths() {
        let mut index = Stmt::new(
            Some(4),
            Op::PcmpStrIndex(PcmpStrIndex {
                lhs: 0,
                rhs: 1,
                lhs_len: Some(2),
                rhs_len: Some(3),
                imm8: 0x18,
            }),
        );
        index.map_refs(|r| r + 20);
        assert_eq!(index.result, Some(24));
        match index.op {
            Op::PcmpStrIndex(p) => {
                assert_eq!(p.lhs, 20);
                assert_eq!(p.rhs, 21);
                assert_eq!(p.lhs_len, Some(22));
                assert_eq!(p.rhs_len, Some(23));
                assert_eq!(p.imm8, 0x18);
            }
            _ => panic!("op changed"),
        }

        let mut mask = Stmt::new(
            Some(12),
            Op::PcmpStrMask(PcmpStrMask {
                lhs: 10,
                rhs: 11,
                lhs_len: None,
                rhs_len: Some(13),
                imm8: 0x40,
            }),
        );
        mask.map_refs(|r| r + 5);
        assert_eq!(mask.result, Some(17));
        match mask.op {
            Op::PcmpStrMask(p) => {
                assert_eq!(p.lhs, 15);
                assert_eq!(p.rhs, 16);
                assert_eq!(p.lhs_len, None);
                assert_eq!(p.rhs_len, Some(18));
                assert_eq!(p.imm8, 0x40);
            }
            _ => panic!("op changed"),
        }

        let mut flags = Stmt::new(
            Some(30),
            Op::PcmpStrFlags(PcmpStrFlags {
                lhs: 20,
                rhs: 21,
                lhs_len: Some(22),
                rhs_len: None,
                imm8: 0x04,
            }),
        );
        flags.map_refs(|r| r + 2);
        assert_eq!(flags.result, Some(32));
        match flags.op {
            Op::PcmpStrFlags(p) => {
                assert_eq!(p.lhs, 22);
                assert_eq!(p.rhs, 23);
                assert_eq!(p.lhs_len, Some(24));
                assert_eq!(p.rhs_len, None);
                assert_eq!(p.imm8, 0x04);
            }
            _ => panic!("op changed"),
        }
    }

    #[test]
    fn map_refs_identity_is_a_noop_and_skips_non_ref_fields() {
        // StoreReg names a register (not a ref) plus a value ref; only the ref
        // moves. Constant has no refs at all.
        let mut store = Stmt::new(
            None,
            Op::StoreReg(StoreReg {
                reg: Gpr::Rax,
                value: 3,
                size: OpSize::I64,
            }),
        );
        let before = store.clone();
        store.map_refs(|r| r); // identity
        assert_eq!(store, before);
        store.map_refs(|r| r + 5);
        match store.op {
            Op::StoreReg(s) => {
                assert_eq!(s.value, 8);
                assert_eq!(s.reg, Gpr::Rax, "register index is not a ref");
            }
            _ => panic!("op changed"),
        }

        let mut konst = Stmt::new(
            Some(0),
            Op::Constant(Constant {
                value: 0xDEAD,
                size: OpSize::I32,
            }),
        );
        konst.map_refs(|r| r + 100);
        assert_eq!(konst.result, Some(100));
        assert!(matches!(konst.op, Op::Constant(c) if c.value == 0xDEAD));
    }

    #[test]
    fn op_size_bit_width() {
        assert_eq!(OpSize::I8.bit_width(), 8);
        assert_eq!(OpSize::I16.bit_width(), 16);
        assert_eq!(OpSize::I32.bit_width(), 32);
        assert_eq!(OpSize::I64.bit_width(), 64);
    }

    #[test]
    fn op_size_masks() {
        assert_eq!(OpSize::I8.mask(), 0xFF);
        assert_eq!(OpSize::I16.mask(), 0xFFFF);
        assert_eq!(OpSize::I32.mask(), 0xFFFF_FFFF);
        assert_eq!(OpSize::I64.mask(), u64::MAX);
    }

    #[test]
    fn constant_op_round_trip() {
        let stmt = Stmt::new(
            Some(0u32),
            Op::Constant(Constant {
                value: 42,
                size: OpSize::I64,
            }),
        );
        assert_eq!(stmt.result, Some(0));
        match &stmt.op {
            Op::Constant(c) => {
                assert_eq!(c.value, 42);
                assert_eq!(c.size, OpSize::I64);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn empty_function_serializes() {
        let fn_ = Function {
            blocks: vec![],
            entry: 0,
        };
        let json = serde_json::to_string(&fn_).unwrap();
        let back: Function = serde_json::from_str(&json).unwrap();
        assert_eq!(fn_, back);
    }

    #[test]
    fn binop_round_trip() {
        let stmts = vec![
            Stmt::new(
                Some(0),
                Op::Constant(Constant {
                    value: 10,
                    size: OpSize::I32,
                }),
            ),
            Stmt::new(
                Some(1),
                Op::Constant(Constant {
                    value: 3,
                    size: OpSize::I32,
                }),
            ),
            Stmt::new(
                Some(2),
                Op::BinOp(BinOp {
                    op: BinOpKind::Add,
                    lhs: 0,
                    rhs: 1,
                    size: OpSize::I32,
                }),
            ),
        ];
        let json = serde_json::to_string(&stmts).unwrap();
        let back: Vec<Stmt> = serde_json::from_str(&json).unwrap();
        assert_eq!(stmts.len(), back.len());
        for (a, b) in stmts.iter().zip(back.iter()) {
            assert_eq!(a, b);
        }
    }

    #[test]
    fn mask_to_size_works() {
        assert_eq!(mask_to_size(0xFF, OpSize::I8), 0xFF);
        assert_eq!(mask_to_size(0x1FFFF, OpSize::I16), 0xFFFF);
        assert_eq!(mask_to_size(0x1_0000_0000, OpSize::I32), 0);
        assert_eq!(mask_to_size(0xDEAD_BEEF, OpSize::I64), 0xDEAD_BEEF);
    }

    #[test]
    fn gpr_count() {
        assert_eq!(GPR_COUNT, 16);
    }

    #[test]
    fn all_op_variants_display() {
        // Quick structural check that every variant constructible.
        fn check(op: &Op) {
            let _json = serde_json::to_string(&op).unwrap();
        }
        check(&Op::Return(Return));
        check(&Op::Cpuid(Cpuid));
        check(&Op::Xgetbv(Xgetbv));
        check(&Op::Rdtsc(Rdtsc));
        check(&Op::Syscall(Syscall));
        check(&Op::X87Pop(X87Pop));
    }
}
