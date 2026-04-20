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
        case ir::BinOpKind::Mul: em.mul(rd, rn, rm);  return {};
        case ir::BinOpKind::And: em.and_(rd, rn, rm); return {};
        case ir::BinOpKind::Or:  em.orr(rd, rn, rm);  return {};
        case ir::BinOpKind::Xor: em.eor(rd, rn, rm);  return {};
        case ir::BinOpKind::Shl: em.lsl(rd, rn, rm);  return {};
        case ir::BinOpKind::Shr: em.lsr(rd, rn, rm);  return {};
        case ir::BinOpKind::Sar: em.asr(rd, rn, rm);  return {};
        case ir::BinOpKind::Ror: em.ror(rd, rn, rm);  return {};
        case ir::BinOpKind::Rcr: em.ror(rd, rn, rm);  return {};
        case ir::BinOpKind::Rol:
        case ir::BinOpKind::Rcl:
            // Rol/Rcl are lowered via a neg+ror helper register in lower_stmt.
            return {false, LowerError::UnsupportedOp,
                    "rotate-left emulation requires a temporary scratch register"};
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

bool Lowerer::allocate_temporary(arm64::Reg& out) {
    if (next_scratch_ >= kScratchPoolSize) return false;
    out = scratch_reg(next_scratch_++);
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
            // Rotate-left and rotate-left-with-carry are emulated as:
            //   neg tmp, rm; ror rd, rn, tmp
            // This keeps us progressing with minimal IR extensions while preserving
            // the current register-only backend.
            if (op.op == ir::BinOpKind::Rol || op.op == ir::BinOpKind::Rcl) {
                arm64::Reg tmp;
                if (!allocate_temporary(tmp)) {
                    return {false, LowerError::OutOfScratchRegs, "BinOp temporary"};
                }
                emitter_.neg(tmp, rm);
                emitter_.ror(rd, rn, tmp);
                return {};
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
        else if constexpr (std::is_same_v<T, ir::LoadMem>) {
            if (!s.result) return {false, LowerError::DanglingRef, "LoadMem without result ref"};
            arm64::Reg raddr;
            if (!reg_of(op.addr, raddr)) return {false, LowerError::DanglingRef, "LoadMem.addr"};
            arm64::Reg rd;
            if (!allocate_scratch(*s.result, rd)) {
                return {false, LowerError::OutOfScratchRegs, "LoadMem"};
            }
            emitter_.load(rd, raddr, op.size);
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::StoreMem>) {
            arm64::Reg raddr, rv;
            if (!reg_of(op.addr, raddr)) return {false, LowerError::DanglingRef, "StoreMem.addr"};
            if (!reg_of(op.value, rv))   return {false, LowerError::DanglingRef, "StoreMem.value"};
            emitter_.store(rv, raddr, op.size);
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::LoadMemTSO>) {
            if (!s.result) return {false, LowerError::DanglingRef, "LoadMemTSO without result ref"};
            arm64::Reg raddr;
            if (!reg_of(op.addr, raddr)) return {false, LowerError::DanglingRef, "LoadMemTSO.addr"};
            arm64::Reg rd;
            if (!allocate_scratch(*s.result, rd)) {
                return {false, LowerError::OutOfScratchRegs, "LoadMemTSO"};
            }
            emitter_.load_acquire(rd, raddr, op.size);
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::StoreMemTSO>) {
            arm64::Reg raddr, rv;
            if (!reg_of(op.addr, raddr)) return {false, LowerError::DanglingRef, "StoreMemTSO.addr"};
            if (!reg_of(op.value, rv))   return {false, LowerError::DanglingRef, "StoreMemTSO.value"};
            emitter_.store_release(rv, raddr, op.size);
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::CmpFlags>) {
            // Side-effecting: emits ARM64 `cmp`, leaves NZCV set for the
            // NEXT CondJumpRel / SetCC. No result ref; no scratch
            // allocated.
            arm64::Reg rl, rr;
            if (!reg_of(op.lhs, rl)) return {false, LowerError::DanglingRef, "CmpFlags.lhs"};
            if (!reg_of(op.rhs, rr)) return {false, LowerError::DanglingRef, "CmpFlags.rhs"};
            emitter_.cmp(rl, rr);
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::JumpRel>) {
            // Load the absolute guest target into x0 and return from the
            // block. The future dispatcher uses x0 as "next guest PC".
            emitter_.mov_imm64(arm64::Reg::X0, op.target_guest_pc);
            emitter_.ret();
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::CondJumpRel>) {
            // Invariant: some earlier op (CmpFlags in MVP) set NZCV and
            // nothing has clobbered it since. movz / movk used by
            // mov_imm64 don't touch flags, so we can use csel safely.
            //
            //   mov x0, fallthrough
            //   mov x1, target
            //   csel x0, x1, x0, <cc>    ; x0 = cc ? target : fallthrough
            //   ret
            emitter_.mov_imm64(arm64::Reg::X0, op.fallthrough_guest_pc);
            emitter_.mov_imm64(arm64::Reg::X1, op.target_guest_pc);
            emitter_.csel(arm64::Reg::X0, arm64::Reg::X1, arm64::Reg::X0, op.cc);
            emitter_.ret();
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::Return>) {
            // Guest RET in MVP: set x0 = 0 (the dispatcher halt sentinel,
            // `CpuStateFrame::kHaltSentinel`) and ret. A real x86 RET pops
            // the return address from the guest stack; that requires a
            // stack and CALL-side pushing. Until F1-RT-008 lands (return-
            // address stack), "guest returned" means "end of execution".
            emitter_.mov_imm64(arm64::Reg::X0, /*kHaltSentinel=*/0);
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
