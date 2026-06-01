// prisma/ir/operation.cpp — IR equality operators.
//
// Separate from pretty-printing so translation units compile faster.

#include "prisma/ir.hpp"

namespace prisma::ir {

bool operator==(const Constant& a, const Constant& b) noexcept {
    return a.value == b.value && a.size == b.size;
}
bool operator==(const LoadReg& a, const LoadReg& b) noexcept {
    return a.reg == b.reg && a.size == b.size;
}
bool operator==(const StoreReg& a, const StoreReg& b) noexcept {
    return a.reg == b.reg && a.value == b.value && a.size == b.size;
}
bool operator==(const LoadSegBase& a, const LoadSegBase& b) noexcept {
    return a.seg == b.seg;
}
bool operator==(const BinOp& a, const BinOp& b) noexcept {
    return a.op == b.op && a.lhs == b.lhs && a.rhs == b.rhs && a.size == b.size;
}
bool operator==(const Compare& a, const Compare& b) noexcept {
    return a.cc == b.cc && a.lhs == b.lhs && a.rhs == b.rhs && a.size == b.size;
}
bool operator==(const Select& a, const Select& b) noexcept {
    return a.cc == b.cc && a.true_value == b.true_value
        && a.false_value == b.false_value && a.size == b.size;
}
bool operator==(const LoadMem& a, const LoadMem& b) noexcept {
    return a.addr == b.addr && a.size == b.size;
}
bool operator==(const StoreMem& a, const StoreMem& b) noexcept {
    return a.addr == b.addr && a.value == b.value && a.size == b.size;
}
bool operator==(const LoadMemTSO& a, const LoadMemTSO& b) noexcept {
    return a.addr == b.addr && a.size == b.size;
}
bool operator==(const StoreMemTSO& a, const StoreMemTSO& b) noexcept {
    return a.addr == b.addr && a.value == b.value && a.size == b.size;
}
bool operator==(const Jump& a, const Jump& b) noexcept {
    return a.target_block == b.target_block;
}
bool operator==(const JumpReg& a, const JumpReg& b) noexcept {
    return a.target == b.target;
}
bool operator==(const CondJump& a, const CondJump& b) noexcept {
    return a.cond == b.cond && a.if_true == b.if_true && a.if_false == b.if_false;
}
bool operator==(const Return&, const Return&) noexcept {
    return true;
}

bool operator==(const CmpFlags& a, const CmpFlags& b) noexcept {
    return a.lhs == b.lhs && a.rhs == b.rhs && a.size == b.size;
}
bool operator==(const JumpRel& a, const JumpRel& b) noexcept {
    return a.target_guest_pc == b.target_guest_pc;
}
bool operator==(const CallRel& a, const CallRel& b) noexcept {
    return a.target_guest_pc == b.target_guest_pc
        && a.return_guest_pc == b.return_guest_pc;
}
bool operator==(const CallReg& a, const CallReg& b) noexcept {
    return a.target == b.target && a.return_guest_pc == b.return_guest_pc;
}
bool operator==(const RetAdjusted& a, const RetAdjusted& b) noexcept {
    return a.pop_bytes == b.pop_bytes;
}
bool operator==(const Cpuid&, const Cpuid&) noexcept {
    return true;
}
bool operator==(const Syscall&, const Syscall&) noexcept {
    return true;
}
bool operator==(const Trap& a, const Trap& b) noexcept {
    return a.kind == b.kind;
}
bool operator==(const Fence& a, const Fence& b) noexcept {
    return a.kind == b.kind;
}
bool operator==(const CondJumpRel& a, const CondJumpRel& b) noexcept {
    return a.cc == b.cc
        && a.target_guest_pc == b.target_guest_pc
        && a.fallthrough_guest_pc == b.fallthrough_guest_pc;
}
bool operator==(const Extend& a, const Extend& b) noexcept {
    return a.value == b.value
        && a.from_size == b.from_size
        && a.to_size == b.to_size
        && a.is_signed == b.is_signed;
}
bool operator==(const Truncate& a, const Truncate& b) noexcept {
    return a.value == b.value && a.to_size == b.to_size;
}
bool operator==(const GuestPc& a, const GuestPc& b) noexcept {
    return a.pc == b.pc;
}
bool operator==(const InlineAsm& a, const InlineAsm& b) noexcept {
    return a.bytes == b.bytes;
}
bool operator==(const FpConstant& a, const FpConstant& b) noexcept {
    return a.bits == b.bits && a.size == b.size;
}
bool operator==(const FpBinOp& a, const FpBinOp& b) noexcept {
    return a.op == b.op && a.lhs == b.lhs && a.rhs == b.rhs && a.size == b.size;
}
bool operator==(const WriteFlags& a, const WriteFlags& b) noexcept {
    return a.op == b.op && a.lhs == b.lhs && a.rhs == b.rhs && a.size == b.size;
}
bool operator==(const ReadFlag& a, const ReadFlag& b) noexcept {
    return a.flags == b.flags && a.which == b.which;
}
bool operator==(const CondJumpFlags& a, const CondJumpFlags& b) noexcept {
    return a.flags == b.flags && a.cc == b.cc
        && a.if_true == b.if_true && a.if_false == b.if_false;
}
bool operator==(const RspAdjust& a, const RspAdjust& b) noexcept {
    return a.delta_bytes == b.delta_bytes;
}
bool operator==(const VecConstant& a, const VecConstant& b) noexcept {
    return a.lo == b.lo && a.hi == b.hi;
}
bool operator==(const VecBinOp& a, const VecBinOp& b) noexcept {
    return a.op == b.op && a.lhs == b.lhs && a.rhs == b.rhs && a.lane == b.lane;
}
bool operator==(const LoadVecReg& a, const LoadVecReg& b) noexcept {
    return a.xmm_index == b.xmm_index;
}
bool operator==(const StoreVecReg& a, const StoreVecReg& b) noexcept {
    return a.xmm_index == b.xmm_index && a.value == b.value;
}
bool operator==(const LoadVecRegHi& a, const LoadVecRegHi& b) noexcept {
    return a.ymm_index == b.ymm_index;
}
bool operator==(const StoreVecRegHi& a, const StoreVecRegHi& b) noexcept {
    return a.ymm_index == b.ymm_index && a.value == b.value;
}
bool operator==(const VecFpFma& a, const VecFpFma& b) noexcept {
    return a.a == b.a && a.b == b.b && a.c == b.c
        && a.neg_addend == b.neg_addend && a.neg_mul == b.neg_mul
        && a.size == b.size;
}
bool operator==(const VecFpScalarFma& a, const VecFpScalarFma& b) noexcept {
    return a.a == b.a && a.b == b.b && a.c == b.c
        && a.scalar_upper == b.scalar_upper
        && a.neg_addend == b.neg_addend && a.neg_mul == b.neg_mul
        && a.size == b.size;
}
bool operator==(const RepStos& a, const RepStos& b) noexcept {
    return a.size == b.size && a.reverse == b.reverse
        && a.pc_of_rep == b.pc_of_rep
        && a.pc_after_rep == b.pc_after_rep;
}
bool operator==(const RepMovs& a, const RepMovs& b) noexcept {
    return a.size == b.size && a.reverse == b.reverse
        && a.pc_of_rep == b.pc_of_rep
        && a.pc_after_rep == b.pc_after_rep;
}
bool operator==(const X87Load& a, const X87Load& b) noexcept {
    return a.st_index == b.st_index;
}
bool operator==(const X87Store& a, const X87Store& b) noexcept {
    return a.st_index == b.st_index && a.value == b.value;
}
bool operator==(const X87Push& a, const X87Push& b) noexcept {
    return a.value == b.value;
}
bool operator==(const X87Pop&, const X87Pop&) noexcept {
    return true;
}
bool operator==(const VecFpBinOp& a, const VecFpBinOp& b) noexcept {
    return a.op == b.op && a.lhs == b.lhs && a.rhs == b.rhs && a.size == b.size;
}
bool operator==(const VecFpScalarBinOp& a, const VecFpScalarBinOp& b) noexcept {
    return a.op == b.op && a.lhs == b.lhs && a.rhs == b.rhs && a.size == b.size;
}
bool operator==(const LoadVec& a, const LoadVec& b) noexcept {
    return a.addr == b.addr;
}
bool operator==(const StoreVec& a, const StoreVec& b) noexcept {
    return a.addr == b.addr && a.value == b.value;
}
bool operator==(const XmmFromGpr& a, const XmmFromGpr& b) noexcept {
    return a.value == b.value && a.size == b.size;
}
bool operator==(const GprFromXmm& a, const GprFromXmm& b) noexcept {
    return a.value == b.value && a.size == b.size;
}
bool operator==(const VecCmp& a, const VecCmp& b) noexcept {
    return a.kind == b.kind && a.lhs == b.lhs && a.rhs == b.rhs && a.lane == b.lane;
}
bool operator==(const VecShuffle32x4& a, const VecShuffle32x4& b) noexcept {
    return a.src == b.src && a.control == b.control;
}
bool operator==(const VecUnpack& a, const VecUnpack& b) noexcept {
    return a.is_high == b.is_high && a.lhs == b.lhs && a.rhs == b.rhs && a.lane == b.lane;
}
bool operator==(const VecShiftImm& a, const VecShiftImm& b) noexcept {
    return a.kind == b.kind && a.src == b.src && a.count == b.count && a.lane == b.lane;
}
bool operator==(const VecShiftBytes& a, const VecShiftBytes& b) noexcept {
    return a.is_left == b.is_left && a.src == b.src && a.count == b.count;
}
bool operator==(const IntToFpScalar& a, const IntToFpScalar& b) noexcept {
    return a.value == b.value && a.int_size == b.int_size && a.fp_size == b.fp_size;
}
bool operator==(const FpToIntScalar& a, const FpToIntScalar& b) noexcept {
    return a.value == b.value && a.fp_size == b.fp_size && a.int_size == b.int_size;
}
bool operator==(const FpCvtScalar& a, const FpCvtScalar& b) noexcept {
    return a.lhs == b.lhs && a.src == b.src
        && a.src_size == b.src_size && a.dst_size == b.dst_size;
}
bool operator==(const VecShuffle2Src& a, const VecShuffle2Src& b) noexcept {
    return a.is_pd == b.is_pd && a.lhs == b.lhs && a.rhs == b.rhs
        && a.control == b.control;
}
bool operator==(const VecInsertLane& a, const VecInsertLane& b) noexcept {
    return a.lhs_xmm == b.lhs_xmm && a.value == b.value
        && a.lane_idx == b.lane_idx && a.lane == b.lane;
}
bool operator==(const VecExtractLaneU& a, const VecExtractLaneU& b) noexcept {
    return a.src_xmm == b.src_xmm && a.lane_idx == b.lane_idx && a.lane == b.lane;
}
bool operator==(const VecMaskMsb& a, const VecMaskMsb& b) noexcept {
    return a.src_xmm == b.src_xmm;
}
bool operator==(const WriteFlagsFp& a, const WriteFlagsFp& b) noexcept {
    return a.lhs == b.lhs && a.rhs == b.rhs && a.size == b.size;
}
bool operator==(const VecShuffleH4& a, const VecShuffleH4& b) noexcept {
    return a.is_high == b.is_high && a.src == b.src && a.control == b.control;
}
bool operator==(const VecMaskFp& a, const VecMaskFp& b) noexcept {
    return a.src_xmm == b.src_xmm && a.is_pd == b.is_pd;
}
bool operator==(const VecFpCompare& a, const VecFpCompare& b) noexcept {
    return a.lhs == b.lhs && a.rhs == b.rhs && a.size == b.size
        && a.pred == b.pred && a.is_packed == b.is_packed;
}
bool operator==(const VecPshufb& a, const VecPshufb& b) noexcept {
    return a.src == b.src && a.mask == b.mask;
}
bool operator==(const VecAbs& a, const VecAbs& b) noexcept {
    return a.src == b.src && a.lane == b.lane;
}
bool operator==(const VecAlignr& a, const VecAlignr& b) noexcept {
    return a.lhs == b.lhs && a.rhs == b.rhs && a.count == b.count;
}
bool operator==(const VecExtend& a, const VecExtend& b) noexcept {
    return a.src == b.src && a.narrow_lane == b.narrow_lane
        && a.wide_lane == b.wide_lane && a.is_signed == b.is_signed;
}
bool operator==(const VecFpRound& a, const VecFpRound& b) noexcept {
    return a.lhs == b.lhs && a.src == b.src && a.size == b.size
        && a.mode == b.mode && a.is_packed == b.is_packed;
}
bool operator==(const Popcnt& a, const Popcnt& b) noexcept {
    return a.value == b.value && a.size == b.size;
}
bool operator==(const Lzcnt& a, const Lzcnt& b) noexcept {
    return a.value == b.value && a.size == b.size;
}
bool operator==(const Tzcnt& a, const Tzcnt& b) noexcept {
    return a.value == b.value && a.size == b.size;
}
bool operator==(const VecBlend& a, const VecBlend& b) noexcept {
    return a.dst == b.dst && a.src == b.src && a.mask == b.mask && a.lane == b.lane;
}
bool operator==(const WriteFlagsPtest& a, const WriteFlagsPtest& b) noexcept {
    return a.lhs == b.lhs && a.rhs == b.rhs;
}
bool operator==(const WriteFlagsPtestYmm& a, const WriteFlagsPtestYmm& b) noexcept {
    return a.lo_lhs == b.lo_lhs && a.lo_rhs == b.lo_rhs
        && a.hi_lhs == b.hi_lhs && a.hi_rhs == b.hi_rhs;
}
bool operator==(const VecTbl2& a, const VecTbl2& b) noexcept {
    return a.src_lo == b.src_lo && a.src_hi == b.src_hi && a.idx == b.idx;
}
bool operator==(const VecAes& a, const VecAes& b) noexcept {
    return a.src == b.src && a.key == b.key && a.kind == b.kind;
}
bool operator==(const VecAesKeygenAssist& a, const VecAesKeygenAssist& b) noexcept {
    return a.src == b.src && a.rcon == b.rcon;
}
bool operator==(const Bswap& a, const Bswap& b) noexcept {
    return a.value == b.value && a.size == b.size;
}
bool operator==(const Crc32c& a, const Crc32c& b) noexcept {
    return a.crc == b.crc && a.data == b.data && a.data_size == b.data_size;
}

bool operator==(const Stmt& a, const Stmt& b) noexcept {
    return a.result == b.result && a.op == b.op;
}

}  // namespace prisma::ir
