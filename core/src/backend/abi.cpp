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
    em.pop_pair(arm64::Reg::X27, arm64::Reg::X8);
    em.pop_pair(arm64::Reg::X9, arm64::Reg::X30);
}

}  // namespace

void emit_block_prologue(Emitter& em) {
    // Six pairs keep the frame 96 bytes and 16-byte aligned. Generated
    // code leaves x28/x29 untouched, so those two saved lanes may hold
    // bounded spill values; the epilogue discards them via x8/x9.
    em.push_pair(arm64::Reg::X29, arm64::Reg::X30);
    em.push_pair(arm64::Reg::X27, arm64::Reg::X28);
    em.push_pair(arm64::Reg::X25, arm64::Reg::X26);
    em.push_pair(arm64::Reg::X23, arm64::Reg::X24);
    em.push_pair(arm64::Reg::X21, arm64::Reg::X22);
    em.push_pair(arm64::Reg::X19, arm64::Reg::X20);

    em.mov_reg_reg(kStatePtrReg, arm64::Reg::X0);

    for (std::size_t i = 0; i < ir::kGprCount; ++i) {
        const ir::Gpr g = static_cast<ir::Gpr>(i);
        const arm64::Reg host = arm64::host_reg_for(g);
        const std::int32_t off = runtime::CpuStateFrame::gpr_offset_bytes(g);
        em.load_offset(host, kStatePtrReg, off);
    }
}

void emit_block_epilogue_and_ret(Emitter& em) {
    store_pinned_guest_gprs(em);
    restore_callee_saved_pairs(em);
    em.ret();
}

PatchableTailEpilogue emit_block_epilogue_patchable_tail(Emitter& em) {
    store_pinned_guest_gprs(em);

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
