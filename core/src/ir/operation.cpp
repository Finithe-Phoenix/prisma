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
bool operator==(const BinOp& a, const BinOp& b) noexcept {
    return a.op == b.op && a.lhs == b.lhs && a.rhs == b.rhs && a.size == b.size;
}
bool operator==(const Compare& a, const Compare& b) noexcept {
    return a.cc == b.cc && a.lhs == b.lhs && a.rhs == b.rhs && a.size == b.size;
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
bool operator==(const CondJump& a, const CondJump& b) noexcept {
    return a.cond == b.cond && a.if_true == b.if_true && a.if_false == b.if_false;
}
bool operator==(const Return&, const Return&) noexcept {
    return true;
}

bool operator==(const Stmt& a, const Stmt& b) noexcept {
    return a.result == b.result && a.op == b.op;
}

}  // namespace prisma::ir
