// core/src/ir/serialization.cpp - deterministic binary Prisma IR writer.

#include "prisma/ir_serialization.hpp"

#include <type_traits>
#include <utility>
#include <variant>

namespace prisma::ir {

namespace {

enum class OpTag : std::uint8_t {
    Constant = 1,
    LoadReg = 2,
    LoadSegBase = 3,
    StoreReg = 4,
    BinOp = 5,
    Extend = 6,
    Truncate = 7,
    Compare = 8,
    Select = 9,
    LoadMem = 10,
    StoreMem = 11,
    LoadMemTSO = 12,
    StoreMemTSO = 13,
    GuestPc = 14,
    Jump = 15,
    CondJump = 16,
    Return = 17,
    JumpReg = 18,
    CmpFlags = 19,
    JumpRel = 20,
    CallRel = 21,
    CallReg = 22,
    RetAdjusted = 23,
    Cpuid = 24,
    Syscall = 25,
    Trap = 26,
    Fence = 27,
    CondJumpRel = 28,
};

class Writer {
public:
    void u8(std::uint8_t value) {
        bytes_.push_back(value);
    }

    void u16(std::uint16_t value) {
        u8(static_cast<std::uint8_t>(value & 0xFFu));
        u8(static_cast<std::uint8_t>((value >> 8u) & 0xFFu));
    }

    void u32(std::uint32_t value) {
        for (unsigned shift = 0; shift < 32; shift += 8) {
            u8(static_cast<std::uint8_t>((value >> shift) & 0xFFu));
        }
    }

    void u64(std::uint64_t value) {
        for (unsigned shift = 0; shift < 64; shift += 8) {
            u8(static_cast<std::uint8_t>((value >> shift) & 0xFFu));
        }
    }

    template <typename E>
    void enum8(E value) {
        static_assert(std::is_enum_v<E>);
        u8(static_cast<std::uint8_t>(value));
    }

    void header(IrBinaryKind kind) {
        bytes_.insert(bytes_.end(), kIrBinaryMagic.begin(), kIrBinaryMagic.end());
        u16(kIrBinaryVersion);
        enum8(kind);
        u8(0);  // reserved flags byte for future deserializers.
    }

    void stmt(const Stmt& stmt) {
        if (stmt.result) {
            u8(1);
            u32(*stmt.result);
        } else {
            u8(0);
        }
        op(stmt.op);
    }

    void root_op(const Op& op) {
        this->op(op);
    }

    void stmt_list(std::span<const Stmt> stmts) {
        u32(static_cast<std::uint32_t>(stmts.size()));
        for (const auto& stmt : stmts) {
            this->stmt(stmt);
        }
    }

    [[nodiscard]] std::vector<std::uint8_t> finish() && {
        return std::move(bytes_);
    }

private:
    void op(const Op& op) {
        std::visit([&](const auto& x) {
            using T = std::decay_t<decltype(x)>;
            if constexpr (std::is_same_v<T, Constant>) {
                enum8(OpTag::Constant);
                u64(x.value);
                enum8(x.size);
            } else if constexpr (std::is_same_v<T, LoadReg>) {
                enum8(OpTag::LoadReg);
                enum8(x.reg);
                enum8(x.size);
            } else if constexpr (std::is_same_v<T, LoadSegBase>) {
                enum8(OpTag::LoadSegBase);
                enum8(x.segment);
            } else if constexpr (std::is_same_v<T, StoreReg>) {
                enum8(OpTag::StoreReg);
                enum8(x.reg);
                u32(x.value);
                enum8(x.size);
            } else if constexpr (std::is_same_v<T, BinOp>) {
                enum8(OpTag::BinOp);
                enum8(x.op);
                u32(x.lhs);
                u32(x.rhs);
                enum8(x.size);
            } else if constexpr (std::is_same_v<T, Extend>) {
                enum8(OpTag::Extend);
                u32(x.value);
                enum8(x.from_size);
                enum8(x.to_size);
                u8(x.is_signed ? 1 : 0);
            } else if constexpr (std::is_same_v<T, Truncate>) {
                enum8(OpTag::Truncate);
                u32(x.value);
                enum8(x.to_size);
            } else if constexpr (std::is_same_v<T, Compare>) {
                enum8(OpTag::Compare);
                enum8(x.cc);
                u32(x.lhs);
                u32(x.rhs);
                enum8(x.size);
            } else if constexpr (std::is_same_v<T, Select>) {
                enum8(OpTag::Select);
                enum8(x.cc);
                u32(x.true_value);
                u32(x.false_value);
                enum8(x.size);
            } else if constexpr (std::is_same_v<T, LoadMem>) {
                enum8(OpTag::LoadMem);
                u32(x.addr);
                enum8(x.size);
            } else if constexpr (std::is_same_v<T, StoreMem>) {
                enum8(OpTag::StoreMem);
                u32(x.addr);
                u32(x.value);
                enum8(x.size);
            } else if constexpr (std::is_same_v<T, LoadMemTSO>) {
                enum8(OpTag::LoadMemTSO);
                u32(x.addr);
                enum8(x.size);
            } else if constexpr (std::is_same_v<T, StoreMemTSO>) {
                enum8(OpTag::StoreMemTSO);
                u32(x.addr);
                u32(x.value);
                enum8(x.size);
            } else if constexpr (std::is_same_v<T, GuestPc>) {
                enum8(OpTag::GuestPc);
                u64(x.pc);
            } else if constexpr (std::is_same_v<T, Jump>) {
                enum8(OpTag::Jump);
                u32(x.target_block);
            } else if constexpr (std::is_same_v<T, CondJump>) {
                enum8(OpTag::CondJump);
                u32(x.cond);
                u32(x.if_true);
                u32(x.if_false);
            } else if constexpr (std::is_same_v<T, Return>) {
                enum8(OpTag::Return);
            } else if constexpr (std::is_same_v<T, JumpReg>) {
                enum8(OpTag::JumpReg);
                u32(x.target);
            } else if constexpr (std::is_same_v<T, CmpFlags>) {
                enum8(OpTag::CmpFlags);
                u32(x.lhs);
                u32(x.rhs);
                enum8(x.size);
            } else if constexpr (std::is_same_v<T, JumpRel>) {
                enum8(OpTag::JumpRel);
                u64(x.target_guest_pc);
            } else if constexpr (std::is_same_v<T, CallRel>) {
                enum8(OpTag::CallRel);
                u64(x.target_guest_pc);
                u64(x.return_guest_pc);
            } else if constexpr (std::is_same_v<T, CallReg>) {
                enum8(OpTag::CallReg);
                u32(x.target);
                u64(x.return_guest_pc);
            } else if constexpr (std::is_same_v<T, RetAdjusted>) {
                enum8(OpTag::RetAdjusted);
                u32(x.pop_bytes);
            } else if constexpr (std::is_same_v<T, Cpuid>) {
                enum8(OpTag::Cpuid);
            } else if constexpr (std::is_same_v<T, Syscall>) {
                enum8(OpTag::Syscall);
            } else if constexpr (std::is_same_v<T, Trap>) {
                enum8(OpTag::Trap);
                enum8(x.kind);
            } else if constexpr (std::is_same_v<T, Fence>) {
                enum8(OpTag::Fence);
                enum8(x.kind);
            } else if constexpr (std::is_same_v<T, CondJumpRel>) {
                enum8(OpTag::CondJumpRel);
                enum8(x.cc);
                u64(x.target_guest_pc);
                u64(x.fallthrough_guest_pc);
            }
        }, op);
    }

    std::vector<std::uint8_t> bytes_;
};

}  // namespace

std::vector<std::uint8_t> serialize_stmts(std::span<const Stmt> stmts) {
    Writer writer;
    writer.header(IrBinaryKind::StmtList);
    writer.stmt_list(stmts);
    return std::move(writer).finish();
}

std::vector<std::uint8_t> serialize_op(const Op& op) {
    Writer writer;
    writer.root_op(op);
    return std::move(writer).finish();
}

std::vector<std::uint8_t> serialize_function(const Function& function) {
    Writer writer;
    writer.header(IrBinaryKind::Function);
    writer.u32(function.entry);
    writer.u32(static_cast<std::uint32_t>(function.blocks.size()));
    for (const auto& block : function.blocks) {
        writer.u32(block.id);
        writer.stmt_list(block.stmts);
    }
    return std::move(writer).finish();
}

}  // namespace prisma::ir
