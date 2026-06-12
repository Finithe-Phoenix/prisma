// core/src/backend/abi.cpp — implementation of the AAPCS64 helpers.
//
// See `prisma/abi.hpp` for the rationale and the calling convention.

#include "prisma/abi.hpp"

#include "prisma/cpu_state.hpp"
#include "prisma/ir.hpp"

namespace prisma::backend::abi {

namespace {

void store_pinned_guest_gprs(Emitter& em) {
    for (std::size_t i = 0; i < ir::kGprCount; ++i) {
        const ir::Gpr g = static_cast<ir::Gpr>(i);
        const arm64::Reg host = arm64::host_reg_for(g);
        const std::int32_t off = runtime::CpuStateFrame::gpr_offset_bytes(g);
        em.store_offset(host, kStatePtrReg, off);
    }
}

void restore_callee_saved_pairs(Emitter& em) {
    em.pop_pair(arm64::Reg::X19, arm64::Reg::X20);
    em.pop_pair(arm64::Reg::X21, arm64::Reg::X22);
    em.pop_pair(arm64::Reg::X23, arm64::Reg::X24);
    em.pop_pair(arm64::Reg::X25, arm64::Reg::X26);
    em.pop_pair(arm64::Reg::X27, arm64::Reg::X28);
    em.pop_pair(arm64::Reg::X29, arm64::Reg::X30);
}

}  // namespace

void emit_block_prologue(Emitter& em) {
    // Push 6 callee-saved register pairs. Each push_pair emits
    // `stp r1, r2, [sp, #-16]!`; six pairs = 96 bytes, 16-byte aligned.
    // Order is: FP/LR first (closest to the original SP), then state-ptr
    // holder + spare, then the four guest-GPR pairs r8..r15. Reverse
    // order on pop is important for AAPCS64 unwinding.
    em.push_pair(arm64::Reg::X29, arm64::Reg::X30);
    em.push_pair(arm64::Reg::X27, arm64::Reg::X28);
    em.push_pair(arm64::Reg::X25, arm64::Reg::X26);
    em.push_pair(arm64::Reg::X23, arm64::Reg::X24);
    em.push_pair(arm64::Reg::X21, arm64::Reg::X22);
    em.push_pair(arm64::Reg::X19, arm64::Reg::X20);

    // Stash the state pointer (host arg0) in our pinned holder. Anything
    // in x0 below this point is the body's to scribble on.
    em.mov_reg_reg(kStatePtrReg, arm64::Reg::X0);

    // Load each guest GPR from `state->gpr[i]` into its pinned host reg.
    for (std::size_t i = 0; i < ir::kGprCount; ++i) {
        const ir::Gpr g = static_cast<ir::Gpr>(i);
        const arm64::Reg host = arm64::host_reg_for(g);
        const std::int32_t off = runtime::CpuStateFrame::gpr_offset_bytes(g);
        em.load_offset(host, kStatePtrReg, off);
    }
}

void emit_block_epilogue_and_ret(Emitter& em) {
    // Store pinned host regs back into the state frame so the next
    // dispatcher round trip sees the up-to-date guest GPRs.
    store_pinned_guest_gprs(em);

    // Restore callee-saved regs in reverse push order.
    restore_callee_saved_pairs(em);

    em.ret();
}

PatchableTailEpilogue emit_block_epilogue_patchable_tail(Emitter& em) {
    store_pinned_guest_gprs(em);

    // Keep the normal return value available for the unpatched fallback,
    // while preparing x0 for a patched branch to the successor block.
    em.mov_reg_reg(arm64::Reg::X1, arm64::Reg::X0);
    em.mov_reg_reg(arm64::Reg::X0, kStatePtrReg);

    restore_callee_saved_pairs(em);

    auto fallback = em.create_label();
    const std::size_t branch_offset = em.current_offset();
    em.branch(fallback);
    em.bind(fallback);
    const std::size_t fallback_offset = em.current_offset();
    em.mov_reg_reg(arm64::Reg::X0, arm64::Reg::X1);
    em.ret();

    return PatchableTailEpilogue{branch_offset, fallback_offset};
}

}  // namespace prisma::backend::abi
