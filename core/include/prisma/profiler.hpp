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

#include <array>
#include <cstdint>
#include <span>

#include "prisma/ir.hpp"

namespace prisma::ir {

class OpCounter {
public:
    enum class Kind : std::uint8_t {
        Constant = 0, LoadReg, StoreReg, LoadSegBase,
        BinOp, Compare, Select,
        LoadMem, StoreMem, LoadMemTSO, StoreMemTSO,
        Jump, CondJump, Return,
        JumpReg, CmpFlags, JumpRel, CondJumpRel,
        CallRel, CallReg, RetAdjusted,
        Cpuid, Syscall, Trap,
        Extend, Truncate, Fence,
        GuestPc, InlineAsm,
        FpConstant, FpBinOp,
        WriteFlags, ReadFlag, CondJumpFlags,
        RspAdjust,
        VecConstant, VecBinOp,
        LoadVecReg, StoreVecReg,
        VecFpBinOp, VecFpScalarBinOp,
        LoadVec, StoreVec,
        XmmFromGpr, GprFromXmm,
        VecCmp, VecShuffle32x4,
        VecUnpack, VecShiftImm, VecShiftBytes,
        IntToFpScalar, FpToIntScalar, FpCvtScalar,
        VecShuffle2Src,
        VecInsertLane, VecExtractLaneU,
        VecMaskMsb,
        WriteFlagsFp,
        VecShuffleH4,
        VecMaskFp,
        VecFpCompare,
        VecPshufb, VecAbs,
        VecAlignr, VecExtend, VecFpRound,
        Popcnt, Lzcnt, Tzcnt,
        VecBlend,
        WriteFlagsPtest,
        LoadVecRegHi, StoreVecRegHi,
        VecFpFma,
        VecFpScalarFma,
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
    [[nodiscard]] std::array<std::uint64_t,
                             static_cast<std::size_t>(Kind::kCount)>
    snapshot() const noexcept;

private:
    std::array<std::uint64_t, static_cast<std::size_t>(Kind::kCount)>
        counts_{};
};

}  // namespace prisma::ir
