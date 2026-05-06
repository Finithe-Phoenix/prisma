// core/src/ir/profiler.cpp — F1-IR-020.

#include "prisma/profiler.hpp"

#include <variant>

namespace prisma::ir {

namespace {

OpCounter::Kind kind_for(const Op& op) noexcept {
    return std::visit([](const auto& x) -> OpCounter::Kind {
        using T = std::decay_t<decltype(x)>;
        if      constexpr (std::is_same_v<T, Constant>)    return OpCounter::Kind::Constant;
        else if constexpr (std::is_same_v<T, LoadReg>)     return OpCounter::Kind::LoadReg;
        else if constexpr (std::is_same_v<T, StoreReg>)    return OpCounter::Kind::StoreReg;
        else if constexpr (std::is_same_v<T, LoadSegBase>) return OpCounter::Kind::LoadSegBase;
        else if constexpr (std::is_same_v<T, BinOp>)       return OpCounter::Kind::BinOp;
        else if constexpr (std::is_same_v<T, Compare>)     return OpCounter::Kind::Compare;
        else if constexpr (std::is_same_v<T, Select>)      return OpCounter::Kind::Select;
        else if constexpr (std::is_same_v<T, LoadMem>)     return OpCounter::Kind::LoadMem;
        else if constexpr (std::is_same_v<T, StoreMem>)    return OpCounter::Kind::StoreMem;
        else if constexpr (std::is_same_v<T, LoadMemTSO>)  return OpCounter::Kind::LoadMemTSO;
        else if constexpr (std::is_same_v<T, StoreMemTSO>) return OpCounter::Kind::StoreMemTSO;
        else if constexpr (std::is_same_v<T, Jump>)        return OpCounter::Kind::Jump;
        else if constexpr (std::is_same_v<T, CondJump>)    return OpCounter::Kind::CondJump;
        else if constexpr (std::is_same_v<T, Return>)      return OpCounter::Kind::Return;
        else if constexpr (std::is_same_v<T, JumpReg>)     return OpCounter::Kind::JumpReg;
        else if constexpr (std::is_same_v<T, CmpFlags>)    return OpCounter::Kind::CmpFlags;
        else if constexpr (std::is_same_v<T, JumpRel>)     return OpCounter::Kind::JumpRel;
        else if constexpr (std::is_same_v<T, CondJumpRel>) return OpCounter::Kind::CondJumpRel;
        else if constexpr (std::is_same_v<T, CallRel>)     return OpCounter::Kind::CallRel;
        else if constexpr (std::is_same_v<T, CallReg>)     return OpCounter::Kind::CallReg;
        else if constexpr (std::is_same_v<T, RetAdjusted>) return OpCounter::Kind::RetAdjusted;
        else if constexpr (std::is_same_v<T, Cpuid>)       return OpCounter::Kind::Cpuid;
        else if constexpr (std::is_same_v<T, Syscall>)     return OpCounter::Kind::Syscall;
        else if constexpr (std::is_same_v<T, Trap>)        return OpCounter::Kind::Trap;
        else if constexpr (std::is_same_v<T, Extend>)      return OpCounter::Kind::Extend;
        else if constexpr (std::is_same_v<T, Truncate>)    return OpCounter::Kind::Truncate;
        else if constexpr (std::is_same_v<T, Fence>)       return OpCounter::Kind::Fence;
        else if constexpr (std::is_same_v<T, GuestPc>)     return OpCounter::Kind::GuestPc;
        else if constexpr (std::is_same_v<T, InlineAsm>)   return OpCounter::Kind::InlineAsm;
        else if constexpr (std::is_same_v<T, FpConstant>)  return OpCounter::Kind::FpConstant;
        else if constexpr (std::is_same_v<T, FpBinOp>)     return OpCounter::Kind::FpBinOp;
        else if constexpr (std::is_same_v<T, WriteFlags>)    return OpCounter::Kind::WriteFlags;
        else if constexpr (std::is_same_v<T, ReadFlag>)      return OpCounter::Kind::ReadFlag;
        else if constexpr (std::is_same_v<T, CondJumpFlags>) return OpCounter::Kind::CondJumpFlags;
        else if constexpr (std::is_same_v<T, RspAdjust>)     return OpCounter::Kind::RspAdjust;
        else if constexpr (std::is_same_v<T, VecConstant>)   return OpCounter::Kind::VecConstant;
        else if constexpr (std::is_same_v<T, VecBinOp>)      return OpCounter::Kind::VecBinOp;
        else if constexpr (std::is_same_v<T, LoadVecReg>)    return OpCounter::Kind::LoadVecReg;
        else if constexpr (std::is_same_v<T, StoreVecReg>)   return OpCounter::Kind::StoreVecReg;
        else if constexpr (std::is_same_v<T, VecFpBinOp>)    return OpCounter::Kind::VecFpBinOp;
        else if constexpr (std::is_same_v<T, VecFpScalarBinOp>) return OpCounter::Kind::VecFpScalarBinOp;
        else if constexpr (std::is_same_v<T, LoadVec>)       return OpCounter::Kind::LoadVec;
        else if constexpr (std::is_same_v<T, StoreVec>)      return OpCounter::Kind::StoreVec;
        else if constexpr (std::is_same_v<T, XmmFromGpr>)    return OpCounter::Kind::XmmFromGpr;
        else if constexpr (std::is_same_v<T, GprFromXmm>)    return OpCounter::Kind::GprFromXmm;
        else if constexpr (std::is_same_v<T, VecCmp>)        return OpCounter::Kind::VecCmp;
        else if constexpr (std::is_same_v<T, VecShuffle32x4>) return OpCounter::Kind::VecShuffle32x4;
        else if constexpr (std::is_same_v<T, VecUnpack>)     return OpCounter::Kind::VecUnpack;
        else if constexpr (std::is_same_v<T, VecShiftImm>)   return OpCounter::Kind::VecShiftImm;
        else if constexpr (std::is_same_v<T, VecShiftBytes>) return OpCounter::Kind::VecShiftBytes;
        else if constexpr (std::is_same_v<T, IntToFpScalar>) return OpCounter::Kind::IntToFpScalar;
        else if constexpr (std::is_same_v<T, FpToIntScalar>) return OpCounter::Kind::FpToIntScalar;
        else if constexpr (std::is_same_v<T, FpCvtScalar>)   return OpCounter::Kind::FpCvtScalar;
        else if constexpr (std::is_same_v<T, VecShuffle2Src>) return OpCounter::Kind::VecShuffle2Src;
        else if constexpr (std::is_same_v<T, VecInsertLane>) return OpCounter::Kind::VecInsertLane;
        else if constexpr (std::is_same_v<T, VecExtractLaneU>) return OpCounter::Kind::VecExtractLaneU;
        else if constexpr (std::is_same_v<T, VecMaskMsb>)    return OpCounter::Kind::VecMaskMsb;
        else if constexpr (std::is_same_v<T, WriteFlagsFp>)  return OpCounter::Kind::WriteFlagsFp;
        else if constexpr (std::is_same_v<T, VecShuffleH4>)  return OpCounter::Kind::VecShuffleH4;
        else if constexpr (std::is_same_v<T, VecMaskFp>)     return OpCounter::Kind::VecMaskFp;
        else if constexpr (std::is_same_v<T, VecFpCompare>)  return OpCounter::Kind::VecFpCompare;
        else if constexpr (std::is_same_v<T, VecPshufb>)     return OpCounter::Kind::VecPshufb;
        else if constexpr (std::is_same_v<T, VecAbs>)        return OpCounter::Kind::VecAbs;
        else if constexpr (std::is_same_v<T, VecAlignr>)     return OpCounter::Kind::VecAlignr;
        else if constexpr (std::is_same_v<T, VecExtend>)     return OpCounter::Kind::VecExtend;
    }, op);
}

}  // namespace

void OpCounter::reset() noexcept { counts_ = {}; }

void OpCounter::visit(const Stmt& stmt) noexcept {
    ++counts_[static_cast<std::size_t>(kind_for(stmt.op))];
}

void OpCounter::visit(std::span<const Stmt> stmts) noexcept {
    for (const auto& s : stmts) visit(s);
}

void OpCounter::visit(const BasicBlock& block) noexcept {
    visit(std::span<const Stmt>(block.stmts));
}

void OpCounter::visit(const Function& fn) noexcept {
    for (const auto& b : fn.blocks) visit(b);
}

std::uint64_t OpCounter::count(Kind k) const noexcept {
    return counts_[static_cast<std::size_t>(k)];
}

std::uint64_t OpCounter::total() const noexcept {
    std::uint64_t t = 0;
    for (auto c : counts_) t += c;
    return t;
}

std::array<std::uint64_t, static_cast<std::size_t>(OpCounter::Kind::kCount)>
OpCounter::snapshot() const noexcept {
    return counts_;
}

}  // namespace prisma::ir
