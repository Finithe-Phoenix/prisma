// core/src/backend/lowering.cpp — implementation of the Lowerer.
//
// One-pass forward traversal of the IR. The implementation relies on the
// invariant (from RFC 0001) that every Ref has a unique def preceding its
// uses, so a simple linear allocator is correct for a single basic block.

#include "prisma/lowering.hpp"

#include <variant>

namespace prisma::backend {

namespace {

constexpr unsigned kScratchPoolSize = 10;  // x0..x9

constexpr arm64::Reg scratch_reg(unsigned idx) noexcept {
    // x0 = 0, x1 = 1, ... x9 = 9.
    return static_cast<arm64::Reg>(idx);
}

// Map an IR BinOpKind to the Emitter method. Expressed as a small switch
// rather than a table so the compiler catches missing cases.
LowerResult emit_binop(Emitter& em,
                       ir::BinOpKind op,
                       arm64::Reg rd,
                       arm64::Reg rn,
                       arm64::Reg rm) {
    switch (op) {
        case ir::BinOpKind::Add: em.add(rd, rn, rm);  return {};
        case ir::BinOpKind::Sub: em.sub(rd, rn, rm);  return {};
        case ir::BinOpKind::And: em.and_(rd, rn, rm); return {};
        case ir::BinOpKind::Or:  em.orr(rd, rn, rm);  return {};
        case ir::BinOpKind::Xor: em.eor(rd, rn, rm);  return {};
        case ir::BinOpKind::Shl: em.lsl(rd, rn, rm);  return {};
        case ir::BinOpKind::Shr: em.lsr(rd, rn, rm);  return {};
        case ir::BinOpKind::Sar: em.asr(rd, rn, rm);  return {};
    }
    return {false, LowerError::UnsupportedOp, "unknown BinOpKind"};
}

}  // namespace

bool Lowerer::allocate_scratch(ir::Ref ref, arm64::Reg& out) {
    if (next_scratch_ >= kScratchPoolSize) return false;
    out = scratch_reg(next_scratch_++);
    ref_to_scratch_[ref] = out;
    return true;
}

bool Lowerer::reg_of(ir::Ref ref, arm64::Reg& out) const {
    auto it = ref_to_scratch_.find(ref);
    if (it == ref_to_scratch_.end()) return false;
    out = it->second;
    return true;
}

LowerResult Lowerer::lower(std::span<const ir::Stmt> stmts) {
    for (const auto& s : stmts) {
        LowerResult r = lower_stmt(s);
        if (!r.success) return r;
    }
    return {};
}

LowerResult Lowerer::lower_stmt(const ir::Stmt& s) {
    return std::visit([&](auto const& op) -> LowerResult {
        using T = std::decay_t<decltype(op)>;

        if constexpr (std::is_same_v<T, ir::Constant>) {
            if (!s.result) return {false, LowerError::DanglingRef, "Constant without result ref"};
            arm64::Reg rd;
            if (!allocate_scratch(*s.result, rd)) {
                return {false, LowerError::OutOfScratchRegs, "Constant"};
            }
            emitter_.mov_imm64(rd, ir::mask_to_size(op.value, op.size));
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::LoadReg>) {
            if (!s.result) return {false, LowerError::DanglingRef, "LoadReg without result ref"};
            arm64::Reg rd;
            if (!allocate_scratch(*s.result, rd)) {
                return {false, LowerError::OutOfScratchRegs, "LoadReg"};
            }
            // Copy the pinned host reg into a scratch so subsequent StoreReg
            // writes cannot clobber this value.
            emitter_.mov_reg_reg(rd, arm64::host_reg_for(op.reg));
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::StoreReg>) {
            arm64::Reg src;
            if (!reg_of(op.value, src)) {
                return {false, LowerError::DanglingRef, "StoreReg.value"};
            }
            emitter_.mov_reg_reg(arm64::host_reg_for(op.reg), src);
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::BinOp>) {
            if (!s.result) return {false, LowerError::DanglingRef, "BinOp without result ref"};
            arm64::Reg rn, rm;
            if (!reg_of(op.lhs, rn)) return {false, LowerError::DanglingRef, "BinOp.lhs"};
            if (!reg_of(op.rhs, rm)) return {false, LowerError::DanglingRef, "BinOp.rhs"};
            arm64::Reg rd;
            if (!allocate_scratch(*s.result, rd)) {
                return {false, LowerError::OutOfScratchRegs, "BinOp"};
            }
            return emit_binop(emitter_, op.op, rd, rn, rm);
        }
        else if constexpr (std::is_same_v<T, ir::Compare>) {
            // Compare produces a 0/1 value in an SSA ref. Lower to
            // cmp + cset. Size-specific comparison is a future concern
            // (we currently do 64-bit comparison regardless of op.size).
            if (!s.result) return {false, LowerError::DanglingRef, "Compare without result ref"};
            arm64::Reg rn, rm;
            if (!reg_of(op.lhs, rn)) return {false, LowerError::DanglingRef, "Compare.lhs"};
            if (!reg_of(op.rhs, rm)) return {false, LowerError::DanglingRef, "Compare.rhs"};
            arm64::Reg rd;
            if (!allocate_scratch(*s.result, rd)) {
                return {false, LowerError::OutOfScratchRegs, "Compare"};
            }
            emitter_.cmp(rn, rm);
            emitter_.cset(rd, op.cc);
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::Return>) {
            // MVP: lower to a bare ARM64 ret. The caller (thunk builder)
            // is responsible for any calling-convention marshalling — the
            // Lowerer just emits the body as the IR describes it.
            emitter_.ret();
            return {};
        }
        else {
            // Compare, Jump, CondJump, LoadMem, StoreMem, LoadMemTSO,
            // StoreMemTSO — deferred to future sessions.
            return {false, LowerError::UnsupportedOp, "op not yet lowered"};
        }
    }, s.op);
}

}  // namespace prisma::backend
