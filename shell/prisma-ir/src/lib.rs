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
pub struct WriteFlagsCountZero {
    pub src: Ref,
    pub result: Ref,
    pub size: OpSize,
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
    Compare(Compare),
    Select(Select),
    LoadMem(LoadMem),
    StoreMem(StoreMem),
    LoadMemTSO(LoadMemTSO),
    StoreMemTSO(StoreMemTSO),
    Jump(Jump),
    CondJump(CondJump),
    Return(Return),
    CmpFlags(CmpFlags),
    AluFlags(AluFlags),
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
    LoadVecReg(LoadVecReg),
    StoreVecReg(StoreVecReg),
    LoadVec(LoadVec),
    StoreVec(StoreVec),
    VecFpBinOp(VecFpBinOp),
    VecFpScalarBinOp(VecFpScalarBinOp),
    XmmFromGpr(XmmFromGpr),
    GprFromXmm(GprFromXmm),
    VecCmp(VecCmp),
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
