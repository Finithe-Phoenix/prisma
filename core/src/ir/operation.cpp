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
bool operator==(const CondJumpRel& a, const CondJumpRel& b) noexcept {
    return a.cc == b.cc
        && a.target_guest_pc == b.target_guest_pc
        && a.fallthrough_guest_pc == b.fallthrough_guest_pc;
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
bool operator==(const Cpuid&, const Cpuid&) noexcept   { return true; }
bool operator==(const Syscall&, const Syscall&) noexcept { return true; }
bool operator==(const Trap& a, const Trap& b) noexcept { return a.kind == b.kind; }
bool operator==(const Extend& a, const Extend& b) noexcept {
    return a.value == b.value
        && a.from_size == b.from_size
        && a.to_size == b.to_size
        && a.is_signed == b.is_signed;
}
bool operator==(const Truncate& a, const Truncate& b) noexcept {
    return a.value == b.value && a.to_size == b.to_size;
}
bool operator==(const Fence& a, const Fence& b) noexcept {
    return a.kind == b.kind;
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

bool operator==(const Stmt& a, const Stmt& b) noexcept {
    return a.result == b.result && a.op == b.op;
}

}  // namespace prisma::ir
