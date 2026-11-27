// prisma/profiler.hpp — F1-IR-020 IR profiler instrumentation.
//
// Tally how often each Op variant appears in a statement list or
// Function. Cheap (one std::visit per stmt), thread-unsafe (intended
// for offline / single-thread analysis). The future ML feature
// pipeline (Pillar 1) reads these counts as input features for the
// "should this region run interpreted vs compiled?" classifier; for
// now the use cases are diagnostics + benchmarks.
//
// Usage:
//
//   ir::OpCounter c;
//   c.visit(stmts);
//   std::cout << c.count(ir::OpCounter::Kind::BinOp) << " BinOps\n";
//
// `OpCounter::Kind` values mirror RFC 0009's OpKind tag table 1:1 so
// downstream consumers can correlate the two.

#pragma once

#include "prisma/ir.hpp"

#include <array>
#include <cstdint>
#include <span>

namespace prisma::ir {

class OpCounter {
 public:
  enum class Kind : std::uint8_t {
    Constant = 0,
    LoadReg,
    StoreReg,
    LoadSegBase,
    LoadCarry,
    StoreCarry,
    LoadRflags,
    StoreRflags,
    StoreRflagsFromNzcv,
    StoreRflagsFromBits,
    BinOp,
    WideDiv,
    Compare,
    Select,
    LoadMem,
    StoreMem,
    LoadMemTSO,
    StoreMemTSO,
    AtomicCmpxchg,
    AtomicCmpxchgPair,
    Jump,
    CondJump,
    Return,
    JumpReg,
    CmpFlags,
    AluFlags,
    JumpRel,
    CondJumpRel,
    CallRel,
    CallReg,
    RetAdjusted,
    Cpuid,
    Syscall,
    Trap,
    TrapIf,
    Extend,
    Truncate,
    Fence,
    GuestPc,
    InlineAsm,
    FpConstant,
    FpBinOp,
    WriteFlags,
    ReadFlag,
    CondJumpFlags,
    RspAdjust,
    VecConstant,
    VecBinOp,
    VecClMul,
    VecF16Cvt,
    LoadVecReg,
    StoreVecReg,
    VecFpBinOp,
    VecFpScalarBinOp,
    LoadVec,
    StoreVec,
    PcmpStrIndex,
    PcmpStrMask,
    PcmpStrFlags,
    XmmFromGpr,
    GprFromXmm,
    VecCmp,
    VecShuffle32x4,
    VecUnpack,
    VecShiftImm,
    VecShiftBytes,
    IntToFpScalar,
    FpToIntScalar,
    FpCvtScalar,
    VecShuffle2Src,
    VecInsertLane,
    VecExtractLaneU,
    VecMaskMsb,
    WriteFlagsFp,
    VecShuffleH4,
    VecMaskFp,
    VecFpCompare,
    VecPshufb,
    VecAbs,
    VecAlignr,
    VecExtend,
    VecFpRound,
    Popcnt,
    Lzcnt,
    Tzcnt,
    WriteFlagsCountZero,
    VecBlend,
    WriteFlagsPtest,
    LoadVecRegHi,
    StoreVecRegHi,
    VecFpFma,
    VecFpScalarFma,
    RepStos,
    RepMovs,
    WriteFlagsPtestYmm,
    VecTbl2,
    VecAes,
    Bswap,
    Crc32c,
    X87Load,
    X87Store,
    X87Push,
    X87Pop,
    VecAesKeygenAssist,
    VecGather,
    VecSha,
    Xgetbv,
    Rdtsc,
    kCount  // sentinel
  };

  void reset() noexcept;
  void visit(const Stmt& stmt) noexcept;
  void visit(std::span<const Stmt> stmts) noexcept;
  void visit(const BasicBlock& block) noexcept;
  void visit(const Function& fn) noexcept;

  [[nodiscard]] std::uint64_t count(Kind k) const noexcept;
  [[nodiscard]] std::uint64_t total() const noexcept;

  // For diagnostics / tests. Returns a fresh array; the underlying
  // counters are unchanged.
  [[nodiscard]] std::array<std::uint64_t, static_cast<std::size_t>(Kind::kCount)>
  snapshot() const noexcept;

 private:
  std::array<std::uint64_t, static_cast<std::size_t>(Kind::kCount)> counts_{};
};

}  // namespace prisma::ir
