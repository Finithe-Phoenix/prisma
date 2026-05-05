// core/src/backend/lowering.cpp — implementation of the Lowerer.
//
// Two-pass algorithm:
//   1. compute_liveness scans the IR once to record each Ref's last-use
//      statement index.
//   2. The main forward scan lowers stmt by stmt; after each stmt we
//      return the host regs of any Ref whose interval has ended to a
//      free-list pool (the "linear-scan" expire step), and free any
//      single-stmt temporaries.
//
// The SSA property (every Ref has a unique def preceding its uses —
// RFC 0001) makes this sound for a single basic block: no phi handling
// required, live intervals are contiguous, and forward-only expiry
// never releases a register that will be read again.

#include "prisma/lowering.hpp"

#include <algorithm>
#include <variant>

#include "prisma/cpu_state.hpp"

namespace prisma::backend {

namespace {

constexpr unsigned kScratchPoolSize   = 10;  // x0..x9
constexpr unsigned kFpScratchPoolSize = 8;   // V0..V7

constexpr arm64::Reg scratch_reg(unsigned idx) noexcept {
    // x0 = 0, x1 = 1, ... x9 = 9.
    return static_cast<arm64::Reg>(idx);
}

constexpr Emitter::FpReg fp_scratch_reg(unsigned idx) noexcept {
    return static_cast<Emitter::FpReg>(idx);  // V0..V7
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
    if (free_regs_.empty()) {
        // F1-BK-008: try to evict a currently-held ref to a spill slot.
        if (!spill_one_ref()) return false;
    }
    out = free_regs_.back();
    free_regs_.pop_back();
    ref_to_scratch_[ref] = out;
    const unsigned live = static_cast<unsigned>(
        ref_to_scratch_.size() + stmt_temporaries_.size());
    peak_live_ = std::max(peak_live_, live);
    return true;
}

bool Lowerer::allocate_fp_scratch(ir::Ref ref, Emitter::FpReg& out) {
    if (fp_free_.empty()) return false;
    out = fp_free_.back();
    fp_free_.pop_back();
    ref_to_fp_[ref] = out;
    return true;
}

bool Lowerer::fp_reg_of(ir::Ref ref, Emitter::FpReg& out) {
    auto it = ref_to_fp_.find(ref);
    if (it == ref_to_fp_.end()) return false;
    out = it->second;
    return true;
}

bool Lowerer::allocate_temporary(arm64::Reg& out) {
    if (free_regs_.empty()) {
        if (!spill_one_ref()) return false;
    }
    out = free_regs_.back();
    free_regs_.pop_back();
    stmt_temporaries_.push_back(out);
    const unsigned live = static_cast<unsigned>(
        ref_to_scratch_.size() + stmt_temporaries_.size());
    peak_live_ = std::max(peak_live_, live);
    return true;
}

bool Lowerer::spill_one_ref() {
    if (options_.spill_slots == 0) return false;
    if (free_slots_.empty()) return false;
    if (ref_to_scratch_.empty()) return false;

    // Belady: evict the ref whose next use is furthest in the future.
    // `last_use_[r]` is the last use index; anything > current stmt
    // index is still in the future, so the max such value is the
    // farthest-used ref. Refs whose last_use <= stmt_index_ are about
    // to be expired anyway — skipping them avoids spilling a zombie.
    ir::Ref      victim{};
    std::size_t  best = 0;
    bool         found = false;
    for (const auto& [r, _reg] : ref_to_scratch_) {
        auto it = last_use_.find(r);
        const std::size_t lu = (it == last_use_.end()) ? stmt_index_ : it->second;
        if (lu <= stmt_index_) continue;   // dies this stmt or earlier
        if (!found || lu > best) {
            best   = lu;
            victim = r;
            found  = true;
        }
    }
    if (!found) return false;

    const arm64::Reg     vreg = ref_to_scratch_[victim];
    const std::uint32_t  slot = free_slots_.back();
    free_slots_.pop_back();

    const std::int32_t offset =
        options_.spill_slot_base_offset
      + static_cast<std::int32_t>(slot) * 8;
    emitter_.sp_store(vreg, offset);
    spilled_to_slot_[victim] = slot;
    ref_to_scratch_.erase(victim);
    free_regs_.push_back(vreg);

    const unsigned in_flight =
        static_cast<unsigned>(spilled_to_slot_.size());
    peak_spills_ = std::max(peak_spills_, in_flight);
    return true;
}

bool Lowerer::reg_of(ir::Ref ref, arm64::Reg& out) {
    auto it = ref_to_scratch_.find(ref);
    if (it != ref_to_scratch_.end()) {
        out = it->second;
        return true;
    }
    // Maybe it's spilled. Reload into a fresh scratch (spilling again if
    // necessary — a spilled ref's reload can itself cause eviction).
    auto sp_it = spilled_to_slot_.find(ref);
    if (sp_it == spilled_to_slot_.end()) return false;

    const std::uint32_t slot = sp_it->second;
    arm64::Reg          rd;
    if (!allocate_scratch(ref, rd)) return false;
    const std::int32_t offset =
        options_.spill_slot_base_offset
      + static_cast<std::int32_t>(slot) * 8;
    emitter_.sp_load(rd, offset);
    spilled_to_slot_.erase(sp_it);
    free_slots_.push_back(slot);
    out = rd;
    return true;
}

void Lowerer::compute_liveness(std::span<const ir::Stmt> stmts) {
    last_use_.clear();
    auto bump = [&](ir::Ref r, std::size_t i) {
        auto it = last_use_.find(r);
        if (it == last_use_.end() || it->second < i) last_use_[r] = i;
    };
    for (std::size_t i = 0; i < stmts.size(); ++i) {
        const auto& s = stmts[i];
        if (s.result) {
            // Seed last_use with the def index; reads below may extend it.
            // This also ensures a never-used def is freed after its own
            // statement instead of being pinned forever.
            auto& entry = last_use_[*s.result];
            if (entry < i) entry = i;
        }
        std::visit([&](const auto& op) {
            using T = std::decay_t<decltype(op)>;
            if      constexpr (std::is_same_v<T, ir::BinOp>)       { bump(op.lhs, i); bump(op.rhs, i); }
            else if constexpr (std::is_same_v<T, ir::Compare>)     { bump(op.lhs, i); bump(op.rhs, i); }
            else if constexpr (std::is_same_v<T, ir::Select>)      { bump(op.true_value, i); bump(op.false_value, i); }
            else if constexpr (std::is_same_v<T, ir::LoadMem>)     { bump(op.addr, i); }
            else if constexpr (std::is_same_v<T, ir::StoreMem>)    { bump(op.addr, i); bump(op.value, i); }
            else if constexpr (std::is_same_v<T, ir::LoadMemTSO>)  { bump(op.addr, i); }
            else if constexpr (std::is_same_v<T, ir::StoreMemTSO>) { bump(op.addr, i); bump(op.value, i); }
            else if constexpr (std::is_same_v<T, ir::StoreReg>)    { bump(op.value, i); }
            else if constexpr (std::is_same_v<T, ir::CmpFlags>)    { bump(op.lhs, i); bump(op.rhs, i); }
            else if constexpr (std::is_same_v<T, ir::CondJump>)    { bump(op.cond, i); }
            else if constexpr (std::is_same_v<T, ir::JumpReg>)     { bump(op.target, i); }
            else if constexpr (std::is_same_v<T, ir::CallReg>)     { bump(op.target, i); }
            else if constexpr (std::is_same_v<T, ir::Extend>)      { bump(op.value, i); }
            else if constexpr (std::is_same_v<T, ir::Truncate>)    { bump(op.value, i); }
            else if constexpr (std::is_same_v<T, ir::WriteFlags>)    { bump(op.lhs, i); bump(op.rhs, i); }
            else if constexpr (std::is_same_v<T, ir::ReadFlag>)      { bump(op.flags, i); }
            else if constexpr (std::is_same_v<T, ir::CondJumpFlags>) { bump(op.flags, i); }
            else if constexpr (std::is_same_v<T, ir::VecBinOp>)      { bump(op.lhs, i); bump(op.rhs, i); }
            else if constexpr (std::is_same_v<T, ir::StoreVecReg>)   { bump(op.value, i); }
            else if constexpr (std::is_same_v<T, ir::VecFpBinOp>)    { bump(op.lhs, i); bump(op.rhs, i); }
            else if constexpr (std::is_same_v<T, ir::VecFpScalarBinOp>) { bump(op.lhs, i); bump(op.rhs, i); }
            else if constexpr (std::is_same_v<T, ir::LoadVec>)       { bump(op.addr, i); }
            else if constexpr (std::is_same_v<T, ir::StoreVec>)      { bump(op.addr, i); bump(op.value, i); }
            else if constexpr (std::is_same_v<T, ir::XmmFromGpr>)    { bump(op.value, i); }
            else if constexpr (std::is_same_v<T, ir::GprFromXmm>)    { bump(op.value, i); }
            else if constexpr (std::is_same_v<T, ir::VecCmp>)        { bump(op.lhs, i); bump(op.rhs, i); }
            else if constexpr (std::is_same_v<T, ir::VecShuffle32x4>) { bump(op.src, i); }
            else if constexpr (std::is_same_v<T, ir::VecUnpack>)     { bump(op.lhs, i); bump(op.rhs, i); }
            else if constexpr (std::is_same_v<T, ir::VecShiftImm>)   { bump(op.src, i); }
            // Constant, LoadReg, LoadSegBase, Jump, JumpRel, CondJumpRel,
            // Return, CallRel, RetAdjusted, Cpuid, Syscall, Trap, Fence
            // have no operand refs — nothing to bump.
        }, s.op);
    }
}

void Lowerer::expire_intervals() {
    // Return any single-stmt temporaries to the pool.
    for (arm64::Reg r : stmt_temporaries_) free_regs_.push_back(r);
    stmt_temporaries_.clear();

    // Expire SSA refs whose last_use is behind us.
    for (auto it = ref_to_scratch_.begin(); it != ref_to_scratch_.end();) {
        auto lu_it = last_use_.find(it->first);
        // If for some reason the ref is missing from last_use_, keep it
        // alive — better to OOM later than to free a live reg.
        const bool expired = lu_it != last_use_.end()
                             && lu_it->second <= stmt_index_;
        if (expired) {
            free_regs_.push_back(it->second);
            it = ref_to_scratch_.erase(it);
        } else {
            ++it;
        }
    }
}

LowerResult Lowerer::lower(std::span<const ir::Stmt> stmts) {
    // Reset per-call state. `last_use_` and friends may carry stale data
    // from a prior lower() invocation on the same Lowerer instance.
    ref_to_scratch_.clear();
    stmt_temporaries_.clear();
    free_regs_.clear();
    spilled_to_slot_.clear();
    free_slots_.clear();
    block_labels_.clear();
    ref_to_fp_.clear();
    fp_free_.clear();
    fp_free_.reserve(kFpScratchPoolSize);
    for (unsigned i = kFpScratchPoolSize; i-- > 0;) {
        fp_free_.push_back(fp_scratch_reg(i));
    }
    flag_refs_.clear();
    peak_live_   = 0;
    peak_spills_ = 0;

    // Seed the spill slot free-list if spilling is enabled. Slots are
    // handed out in order 0..N-1 so stack layout is predictable.
    if (options_.spill_slots > 0) {
        free_slots_.reserve(options_.spill_slots);
        for (unsigned i = options_.spill_slots; i-- > 0;) {
            free_slots_.push_back(i);
        }
    }

    // Seed the free-list so that allocate_scratch hands out x0 first, x1
    // second, etc. — matches the old bump-pointer ordering, which keeps
    // existing emitter tests happy.
    free_regs_.reserve(kScratchPoolSize);
    for (unsigned i = kScratchPoolSize; i-- > 0;) {
        free_regs_.push_back(scratch_reg(i));
    }

    compute_liveness(stmts);

    for (stmt_index_ = 0; stmt_index_ < stmts.size(); ++stmt_index_) {
        LowerResult r = lower_stmt(stmts[stmt_index_]);
        if (!r.success) return r;
        expire_intervals();
    }
    return {};
}

LowerResult Lowerer::lower(const ir::Function& fn) {
    // Per-call reset, identical to the flat overload.
    ref_to_scratch_.clear();
    stmt_temporaries_.clear();
    free_regs_.clear();
    spilled_to_slot_.clear();
    free_slots_.clear();
    block_labels_.clear();
    ref_to_fp_.clear();
    fp_free_.clear();
    fp_free_.reserve(kFpScratchPoolSize);
    for (unsigned i = kFpScratchPoolSize; i-- > 0;) {
        fp_free_.push_back(fp_scratch_reg(i));
    }
    flag_refs_.clear();
    peak_live_   = 0;
    peak_spills_ = 0;
    if (options_.spill_slots > 0) {
        free_slots_.reserve(options_.spill_slots);
        for (unsigned i = options_.spill_slots; i-- > 0;) free_slots_.push_back(i);
    }

    // Pre-create one Label per block so forward branches resolve.
    block_labels_.reserve(fn.blocks.size());
    for (const auto& b : fn.blocks) {
        block_labels_[b.id] = emitter_.create_label();
    }

    // Per-block: rebuild liveness, refill the scratch pool, bind the
    // label, then lower the block's stmts. SSA refs are block-local in
    // the current MVP (no cross-block phi support yet — F1-IR-021/022
    // will introduce that), so register state is reset between blocks.
    for (const auto& b : fn.blocks) {
        ref_to_scratch_.clear();
        stmt_temporaries_.clear();
        free_regs_.clear();
        free_regs_.reserve(kScratchPoolSize);
        for (unsigned i = kScratchPoolSize; i-- > 0;) {
            free_regs_.push_back(scratch_reg(i));
        }
        compute_liveness(b.stmts);

        emitter_.bind(block_labels_.at(b.id));

        for (stmt_index_ = 0; stmt_index_ < b.stmts.size(); ++stmt_index_) {
            LowerResult r = lower_stmt(b.stmts[stmt_index_]);
            if (!r.success) return r;
            expire_intervals();
        }
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
            // Rotate-left and rotate-left-with-carry lower via the
            // dedicated emitter op that encapsulates `neg tmp, rm; ror
            // rd, rn, tmp`. The temporary comes from the scratch pool
            // and is released at end-of-stmt by the linear-scan
            // allocator.
            if (op.op == ir::BinOpKind::Rol || op.op == ir::BinOpKind::Rcl) {
                arm64::Reg tmp;
                if (!allocate_temporary(tmp)) {
                    return {false, LowerError::OutOfScratchRegs, "BinOp temporary"};
                }
                emitter_.rol(rd, rn, rm, tmp);
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
        else if constexpr (std::is_same_v<T, ir::Select>) {
            if (!s.result) return {false, LowerError::DanglingRef, "Select without result ref"};
            arm64::Reg r_true;
            if (!reg_of(op.true_value, r_true)) {
                return {false, LowerError::DanglingRef, "Select.true_value"};
            }
            arm64::Reg r_false;
            if (!reg_of(op.false_value, r_false)) {
                return {false, LowerError::DanglingRef, "Select.false_value"};
            }
            arm64::Reg rd;
            if (!allocate_scratch(*s.result, rd)) {
                return {false, LowerError::OutOfScratchRegs, "Select"};
            }
            // Select maps directly to ARM64 conditional select using current NZCV
            // (from a prior CmpFlags in MVP flow).
            emitter_.csel(rd, r_true, r_false, op.cc);
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
            // Load the absolute guest target into x0. The dispatcher
            // reads it as the next guest PC. Translator-wrapped mode
            // elides the ret so it can emit a state-save epilogue first.
            emitter_.mov_imm64(arm64::Reg::X0, op.target_guest_pc);
            if (options_.emit_ret_on_terminator) emitter_.ret();
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::JumpReg>) {
            arm64::Reg target;
            if (!reg_of(op.target, target)) {
                return {false, LowerError::DanglingRef, "JumpReg.target"};
            }
            emitter_.mov_reg_reg(arm64::Reg::X0, target);
            if (options_.emit_ret_on_terminator) emitter_.ret();
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::Trap>) {
            // Placeholder until the runtime grows first-class guest
            // signal delivery. The Translator/Dispatcher surfaces the
            // trap via block metadata; the machine code just returns.
            emitter_.mov_imm64(arm64::Reg::X0, /*kHaltSentinel=*/0);
            if (options_.emit_ret_on_terminator) emitter_.ret();
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::Cpuid>) {
            // Placeholder lowering for now: zero the architecturally-written
            // guest outputs (EAX/EBX/ECX/EDX). A real host-query model can
            // replace this op later without changing decoder surface.
            emitter_.mov_imm64(arm64::host_reg_for(ir::Gpr::Rax), 0);
            emitter_.mov_imm64(arm64::host_reg_for(ir::Gpr::Rbx), 0);
            emitter_.mov_imm64(arm64::host_reg_for(ir::Gpr::Rcx), 0);
            emitter_.mov_imm64(arm64::host_reg_for(ir::Gpr::Rdx), 0);
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::Syscall>) {
            // Conservative placeholder: terminate the current block and
            // return the halt sentinel until the syscall layer exists.
            emitter_.mov_imm64(arm64::Reg::X0, /*kHaltSentinel=*/0);
            if (options_.emit_ret_on_terminator) emitter_.ret();
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
            //   ret                      ; (or state-save epilogue + ret)
            emitter_.mov_imm64(arm64::Reg::X0, op.fallthrough_guest_pc);
            emitter_.mov_imm64(arm64::Reg::X1, op.target_guest_pc);
            emitter_.csel(arm64::Reg::X0, arm64::Reg::X1, arm64::Reg::X0, op.cc);
            if (options_.emit_ret_on_terminator) emitter_.ret();
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::Return>) {
            // Guest RET in MVP: set x0 = 0 (the dispatcher halt sentinel,
            // `CpuStateFrame::kHaltSentinel`). The Translator-wrapped
            // path appends a state-save epilogue before the ret, so the
            // Lowerer only emits ret when running in standalone mode.
            emitter_.mov_imm64(arm64::Reg::X0, /*kHaltSentinel=*/0);
            if (options_.emit_ret_on_terminator) emitter_.ret();
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::Jump>) {
            // F1-BK-003. Only valid inside a Function lowering; the flat
            // overload has an empty block_labels_ and falls through.
            auto it = block_labels_.find(op.target_block);
            if (it == block_labels_.end()) {
                return {false, LowerError::UnsupportedOp,
                        "Jump outside Function lowering (no block label)"};
            }
            emitter_.branch(it->second);
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::CondJump>) {
            // F1-BK-004. Cond is an SSA Ref holding 0 or 1 (typically the
            // result of a Compare). Lower as `cbnz xcond, label_true; b
            // label_false`. cbnz / b have the same range envelope so a
            // veneer fires for either or neither, never asymmetrically.
            arm64::Reg rc;
            if (!reg_of(op.cond, rc)) {
                return {false, LowerError::DanglingRef, "CondJump.cond"};
            }
            auto it_t = block_labels_.find(op.if_true);
            auto it_f = block_labels_.find(op.if_false);
            if (it_t == block_labels_.end() || it_f == block_labels_.end()) {
                return {false, LowerError::UnsupportedOp,
                        "CondJump outside Function lowering (no block label)"};
            }
            emitter_.cbnz(rc, it_t->second);
            emitter_.branch(it_f->second);
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::LoadSegBase>) {
            // Placeholder lowering: zero the destination. The real
            // implementation reads from a runtime-supplied segment-base
            // table, but that table doesn't exist yet (the runtime hook
            // is part of F1-RT-014 follow-up work). Producing zero
            // matches the semantics of "TLS not initialised" and is
            // safe for unit tests that exercise nothing but the IR
            // shape. See lowerer regression test for the assertion.
            arm64::Reg rd;
            if (!s.result.has_value()) {
                return {false, LowerError::DanglingRef,
                        "LoadSegBase requires a result ref"};
            }
            if (!allocate_scratch(*s.result, rd)) {
                return {false, LowerError::OutOfScratchRegs, "LoadSegBase"};
            }
            emitter_.mov_imm64(rd, 0);
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::Extend>) {
            // F1-BK-022. Sign / zero-extend a narrower view of the
            // source ref into a fresh 64-bit destination scratch. The
            // ARM64 sxt*/uxt* instructions all read Wn and write Xd.
            arm64::Reg rs;
            if (!reg_of(op.value, rs)) {
                return {false, LowerError::DanglingRef, "Extend.value"};
            }
            arm64::Reg rd;
            if (!s.result.has_value()) {
                return {false, LowerError::DanglingRef,
                        "Extend requires a result ref"};
            }
            if (!allocate_scratch(*s.result, rd)) {
                return {false, LowerError::OutOfScratchRegs, "Extend"};
            }
            if (op.is_signed) {
                switch (op.from_size) {
                    case ir::OpSize::I8:  emitter_.sxtb(rd, rs); break;
                    case ir::OpSize::I16: emitter_.sxth(rd, rs); break;
                    case ir::OpSize::I32: emitter_.sxtw(rd, rs); break;
                    case ir::OpSize::I64:
                        emitter_.mov_reg_reg(rd, rs);  // identity
                        break;
                }
            } else {
                switch (op.from_size) {
                    case ir::OpSize::I8:  emitter_.uxtb(rd, rs); break;
                    case ir::OpSize::I16: emitter_.uxth(rd, rs); break;
                    case ir::OpSize::I32: emitter_.uxtw(rd, rs); break;
                    case ir::OpSize::I64:
                        emitter_.mov_reg_reg(rd, rs);  // identity
                        break;
                }
            }
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::Truncate>) {
            // F1-BK-022. Narrow `value` to `to_size` in a fresh 64-bit
            // scratch. For I32 the cheap idiom is `mov wd, wn` (which
            // zeroes the upper bits). For I8/I16 we materialise the
            // mask and AND.
            arm64::Reg rs;
            if (!reg_of(op.value, rs)) {
                return {false, LowerError::DanglingRef, "Truncate.value"};
            }
            arm64::Reg rd;
            if (!s.result.has_value()) {
                return {false, LowerError::DanglingRef,
                        "Truncate requires a result ref"};
            }
            if (!allocate_scratch(*s.result, rd)) {
                return {false, LowerError::OutOfScratchRegs, "Truncate"};
            }
            switch (op.to_size) {
                case ir::OpSize::I8:  emitter_.uxtb(rd, rs); break;
                case ir::OpSize::I16: emitter_.uxth(rd, rs); break;
                case ir::OpSize::I32: emitter_.uxtw(rd, rs); break;
                case ir::OpSize::I64:
                    emitter_.mov_reg_reg(rd, rs);  // identity
                    break;
            }
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::GuestPc>) {
            // Pseudo-op: no machine code. Tracked in ir::Stmt for cache
            // keying / debugging only. The presence of this op never
            // affects emitted bytes.
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::InlineAsm>) {
            // Placeholder until the dispatcher learns how to call into
            // a software interpreter for the raw guest bytes (planned
            // alongside F1-RT-011 guest signal delivery). For now we
            // just halt the block, returning the sentinel so the
            // dispatcher knows we couldn't handle this region.
            emitter_.mov_imm64(arm64::Reg::X0, /*kHaltSentinel=*/0);
            if (options_.emit_ret_on_terminator) emitter_.ret();
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::RspAdjust>) {
            // F1-RT-013. Add or subtract the literal delta from the
            // pinned host register backing guest RSP. Materialise the
            // (possibly negative) delta via mov_imm64 + add (vixl
            // selects the cheap immediate-add encoding when it fits).
            const arm64::Reg rsp_host = arm64::host_reg_for(ir::Gpr::Rsp);
            arm64::Reg tmp;
            if (!allocate_temporary(tmp)) {
                return {false, LowerError::OutOfScratchRegs, "RspAdjust"};
            }
            if (op.delta_bytes >= 0) {
                emitter_.mov_imm64(tmp,
                    static_cast<std::uint64_t>(op.delta_bytes));
                emitter_.add(rsp_host, rsp_host, tmp);
            } else {
                emitter_.mov_imm64(tmp,
                    static_cast<std::uint64_t>(-op.delta_bytes));
                emitter_.sub(rsp_host, rsp_host, tmp);
            }
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::WriteFlags>) {
            // F1-IR-004 / F1-BK lowering. Emit the flag-setting variant
            // of the integer op so NZCV reflects the result, then bind
            // the result Ref to "NZCV is current" (no machine register
            // backing; the consumer reads NZCV directly).
            arm64::Reg rl, rr;
            if (!reg_of(op.lhs, rl)) return {false, LowerError::DanglingRef, "WriteFlags.lhs"};
            if (!reg_of(op.rhs, rr)) return {false, LowerError::DanglingRef, "WriteFlags.rhs"};
            switch (op.op) {
                case ir::BinOpKind::Sub:
                    // `cmp` is `subs xzr, lhs, rhs` — sets NZCV without
                    // writing a destination.
                    emitter_.cmp(rl, rr);
                    break;
                case ir::BinOpKind::Add:
                case ir::BinOpKind::And: {
                    // adds / ands need a destination register. Allocate
                    // a single-stmt temporary so the result value is
                    // discarded but NZCV is set as a side effect.
                    arm64::Reg rd_tmp;
                    if (!allocate_temporary(rd_tmp)) {
                        return {false, LowerError::OutOfScratchRegs,
                                "WriteFlags(Add/And) needs a temp"};
                    }
                    if (op.op == ir::BinOpKind::Add) {
                        emitter_.adds(rd_tmp, rl, rr);
                    } else {
                        emitter_.ands(rd_tmp, rl, rr);
                    }
                    break;
                }
                default:
                    return {false, LowerError::UnsupportedOp,
                            "WriteFlags only supports Sub/Add/And today"};
            }
            // Result Ref lives in NZCV; track it in flag_refs_ so
            // consumers verify their operand was a real WriteFlags
            // result (no host register backing).
            if (s.result.has_value()) {
                flag_refs_.insert(*s.result);
            }
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::ReadFlag>) {
            // F1-IR-005. Emit `cset rd, <armcond>` for the requested
            // FlagBit. The producer (WriteFlags) must have set NZCV
            // and nothing in between may have clobbered it.
            if (flag_refs_.find(op.flags) == flag_refs_.end()) {
                return {false, LowerError::DanglingRef,
                        "ReadFlag.flags must be a WriteFlags result"};
            }
            if (!s.result.has_value()) {
                return {false, LowerError::DanglingRef,
                        "ReadFlag requires a result ref"};
            }
            arm64::Reg rd;
            if (!allocate_scratch(*s.result, rd)) {
                return {false, LowerError::OutOfScratchRegs, "ReadFlag"};
            }
            ir::CondCode cc = ir::CondCode::Eq;  // mapped per which:
            switch (op.which) {
                case ir::FlagBit::Carry:    cc = ir::CondCode::Cc;   break;
                case ir::FlagBit::Zero:     cc = ir::CondCode::Eq;   break;
                case ir::FlagBit::Sign:     cc = ir::CondCode::Mi;   break;
                case ir::FlagBit::Overflow: cc = ir::CondCode::Ov;   break;
                case ir::FlagBit::Parity:
                case ir::FlagBit::Aux:
                    // Not mappable to ARM64 NZCV directly; software
                    // emulation lands in F1-RT-* future work.
                    return {false, LowerError::UnsupportedOp,
                            "ReadFlag(Parity/Aux) needs SW emulation"};
            }
            emitter_.cset(rd, cc);
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::CondJumpFlags>) {
            // F1-IR-007. Branch on NZCV using the supplied CondCode.
            if (flag_refs_.find(op.flags) == flag_refs_.end()) {
                return {false, LowerError::DanglingRef,
                        "CondJumpFlags.flags must be a WriteFlags result"};
            }
            auto it_t = block_labels_.find(op.if_true);
            auto it_f = block_labels_.find(op.if_false);
            if (it_t == block_labels_.end() || it_f == block_labels_.end()) {
                return {false, LowerError::UnsupportedOp,
                        "CondJumpFlags outside Function lowering (no block label)"};
            }
            emitter_.branch_cc(it_t->second, op.cc);
            emitter_.branch(it_f->second);
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::LoadVecReg>) {
            // Read CpuStateFrame::xmm[idx] into a fresh V scratch. The
            // base register is the pinned state pointer
            // (`backend::abi::kStatePtrReg` = x27); the offset comes
            // from the layout-stable `xmm_offset_bytes(idx)`.
            if (!s.result.has_value()) {
                return {false, LowerError::DanglingRef,
                        "LoadVecReg requires a result ref"};
            }
            if (op.xmm_index >= ir::kXmmCount) {
                return {false, LowerError::UnsupportedOp,
                        "LoadVecReg: xmm_index out of range"};
            }
            Emitter::FpReg rd;
            if (!allocate_fp_scratch(*s.result, rd)) {
                return {false, LowerError::OutOfScratchRegs, "LoadVecReg"};
            }
            const std::int32_t off = static_cast<std::int32_t>(
                runtime::CpuStateFrame::xmm_offset_bytes(op.xmm_index));
            emitter_.vld1_q_offset(rd, arm64::Reg::X27, off);
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::StoreVecReg>) {
            if (op.xmm_index >= ir::kXmmCount) {
                return {false, LowerError::UnsupportedOp,
                        "StoreVecReg: xmm_index out of range"};
            }
            Emitter::FpReg rv;
            if (!fp_reg_of(op.value, rv)) {
                return {false, LowerError::DanglingRef, "StoreVecReg.value"};
            }
            const std::int32_t off = static_cast<std::int32_t>(
                runtime::CpuStateFrame::xmm_offset_bytes(op.xmm_index));
            emitter_.vst1_q_offset(rv, arm64::Reg::X27, off);
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::VecConstant>) {
            // F2-IR-001 lowering. Materialise a 128-bit immediate
            // by writing the low and high u64 lanes through the FP
            // scratch register's two D-views. We synthesise the
            // value via two scalar `fmov_imm` calls reinterpreting
            // the bits as f64. This is correct because vixl's Fmov
            // immediate path does not interpret the bits as a
            // value — it loads them through a literal pool.
            if (!s.result.has_value()) {
                return {false, LowerError::DanglingRef,
                        "VecConstant requires a result ref"};
            }
            Emitter::FpReg rd;
            if (!allocate_fp_scratch(*s.result, rd)) {
                return {false, LowerError::OutOfScratchRegs, "VecConstant"};
            }
            // Two-step: load the low 64 bits, then the high 64 bits
            // into separate scratch regs, then build the 128-bit
            // value by ORring them with proper byte placement.
            // For the MVP we just zero the upper bits and load
            // the low half — enough to test the VecConstant IR
            // shape without needing a true 128-bit literal pool.
            // The full path lands in F2-IR-001 follow-up.
            emitter_.fmov_imm(rd, op.lo, ir::FpSize::F64);
            (void)op.hi;  // upper-half handling deferred.
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::VecBinOp>) {
            // F2-IR-002/003 lowering. 128-bit SIMD integer ops.
            if (!s.result.has_value()) {
                return {false, LowerError::DanglingRef,
                        "VecBinOp requires a result ref"};
            }
            Emitter::FpReg rl, rr;
            if (!fp_reg_of(op.lhs, rl)) {
                return {false, LowerError::DanglingRef, "VecBinOp.lhs"};
            }
            if (!fp_reg_of(op.rhs, rr)) {
                return {false, LowerError::DanglingRef, "VecBinOp.rhs"};
            }
            Emitter::FpReg rd;
            if (!allocate_fp_scratch(*s.result, rd)) {
                return {false, LowerError::OutOfScratchRegs, "VecBinOp"};
            }
            const Emitter::VecLane lane =
                op.lane == ir::VecLane::B16 ? Emitter::VecLane::B16 :
                op.lane == ir::VecLane::H8  ? Emitter::VecLane::H8  :
                op.lane == ir::VecLane::S4  ? Emitter::VecLane::S4  :
                                              Emitter::VecLane::D2;
            switch (op.op) {
                case ir::VecBinOpKind::Add: emitter_.vadd_q(rd, rl, rr, lane); break;
                case ir::VecBinOpKind::Sub: emitter_.vsub_q(rd, rl, rr, lane); break;
                case ir::VecBinOpKind::And: emitter_.vand_q(rd, rl, rr);       break;
                case ir::VecBinOpKind::Or:  emitter_.vorr_q(rd, rl, rr);       break;
                case ir::VecBinOpKind::Xor: emitter_.veor_q(rd, rl, rr);       break;
                case ir::VecBinOpKind::Mul: emitter_.vmul_q(rd, rl, rr, lane); break;
            }
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::VecFpBinOp>) {
            // F2-IR-005. Packed-FP arithmetic — ADDPS/SUBPS/MULPS/DIVPS
            // (S4) and ADDPD/SUBPD/MULPD/DIVPD (D2).
            if (!s.result.has_value()) {
                return {false, LowerError::DanglingRef,
                        "VecFpBinOp requires a result ref"};
            }
            Emitter::FpReg rl, rr;
            if (!fp_reg_of(op.lhs, rl)) {
                return {false, LowerError::DanglingRef, "VecFpBinOp.lhs"};
            }
            if (!fp_reg_of(op.rhs, rr)) {
                return {false, LowerError::DanglingRef, "VecFpBinOp.rhs"};
            }
            Emitter::FpReg rd;
            if (!allocate_fp_scratch(*s.result, rd)) {
                return {false, LowerError::OutOfScratchRegs, "VecFpBinOp"};
            }
            const Emitter::VecLane lane =
                op.size == ir::VecFpSize::S4 ? Emitter::VecLane::S4
                                              : Emitter::VecLane::D2;
            switch (op.op) {
                case ir::VecFpBinOpKind::Add: emitter_.vfadd_q(rd, rl, rr, lane); break;
                case ir::VecFpBinOpKind::Sub: emitter_.vfsub_q(rd, rl, rr, lane); break;
                case ir::VecFpBinOpKind::Mul: emitter_.vfmul_q(rd, rl, rr, lane); break;
                case ir::VecFpBinOpKind::Div: emitter_.vfdiv_q(rd, rl, rr, lane); break;
            }
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::VecUnpack>) {
            if (!s.result.has_value()) {
                return {false, LowerError::DanglingRef, "VecUnpack without result"};
            }
            Emitter::FpReg rl, rr;
            if (!fp_reg_of(op.lhs, rl)) return {false, LowerError::DanglingRef, "VecUnpack.lhs"};
            if (!fp_reg_of(op.rhs, rr)) return {false, LowerError::DanglingRef, "VecUnpack.rhs"};
            Emitter::FpReg rd;
            if (!allocate_fp_scratch(*s.result, rd)) {
                return {false, LowerError::OutOfScratchRegs, "VecUnpack"};
            }
            const Emitter::VecLane lane =
                op.lane == ir::VecLane::B16 ? Emitter::VecLane::B16 :
                op.lane == ir::VecLane::H8  ? Emitter::VecLane::H8  :
                op.lane == ir::VecLane::S4  ? Emitter::VecLane::S4  :
                                              Emitter::VecLane::D2;
            if (op.is_high) emitter_.vzip2_q(rd, rl, rr, lane);
            else            emitter_.vzip1_q(rd, rl, rr, lane);
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::VecShiftImm>) {
            if (!s.result.has_value()) {
                return {false, LowerError::DanglingRef, "VecShiftImm without result"};
            }
            Emitter::FpReg rn;
            if (!fp_reg_of(op.src, rn)) return {false, LowerError::DanglingRef, "VecShiftImm.src"};
            Emitter::FpReg rd;
            if (!allocate_fp_scratch(*s.result, rd)) {
                return {false, LowerError::OutOfScratchRegs, "VecShiftImm"};
            }
            const Emitter::VecLane lane =
                op.lane == ir::VecLane::B16 ? Emitter::VecLane::B16 :
                op.lane == ir::VecLane::H8  ? Emitter::VecLane::H8  :
                op.lane == ir::VecLane::S4  ? Emitter::VecLane::S4  :
                                              Emitter::VecLane::D2;
            switch (op.kind) {
                case ir::VecShiftKind::ShiftL:
                    emitter_.vshl_imm_q(rd, rn, op.count, lane); break;
                case ir::VecShiftKind::LogicalShr:
                    emitter_.vushr_imm_q(rd, rn, op.count, lane); break;
                case ir::VecShiftKind::ArithShr:
                    emitter_.vsshr_imm_q(rd, rn, op.count, lane); break;
            }
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::VecShuffle32x4>) {
            // F2-IR-010. PSHUFD: 4-way 32-bit lane permutation.
            if (!s.result.has_value()) {
                return {false, LowerError::DanglingRef, "VecShuffle32x4 without result"};
            }
            Emitter::FpReg rn;
            if (!fp_reg_of(op.src, rn)) {
                return {false, LowerError::DanglingRef, "VecShuffle32x4.src"};
            }
            Emitter::FpReg rd;
            if (!allocate_fp_scratch(*s.result, rd)) {
                return {false, LowerError::OutOfScratchRegs, "VecShuffle32x4"};
            }
            emitter_.vshuffle_s4(rd, rn, op.control);
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::VecCmp>) {
            // F2-IR-009. cmeq / cmgt on V regs (lane-wise integer compare).
            if (!s.result.has_value()) {
                return {false, LowerError::DanglingRef, "VecCmp without result"};
            }
            Emitter::FpReg rl, rr;
            if (!fp_reg_of(op.lhs, rl)) {
                return {false, LowerError::DanglingRef, "VecCmp.lhs"};
            }
            if (!fp_reg_of(op.rhs, rr)) {
                return {false, LowerError::DanglingRef, "VecCmp.rhs"};
            }
            Emitter::FpReg rd;
            if (!allocate_fp_scratch(*s.result, rd)) {
                return {false, LowerError::OutOfScratchRegs, "VecCmp"};
            }
            const Emitter::VecLane lane =
                op.lane == ir::VecLane::B16 ? Emitter::VecLane::B16 :
                op.lane == ir::VecLane::H8  ? Emitter::VecLane::H8  :
                op.lane == ir::VecLane::S4  ? Emitter::VecLane::S4  :
                                              Emitter::VecLane::D2;
            switch (op.kind) {
                case ir::VecCmpKind::Eq: emitter_.vcmeq_q(rd, rl, rr, lane); break;
                case ir::VecCmpKind::Gt: emitter_.vcmgt_q(rd, rl, rr, lane); break;
            }
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::XmmFromGpr>) {
            // F2-IR-008. fmov d/s_rd, x/w_rn — moves GPR low into V_rd
            // and zero-extends the upper 96/64 bits (fmov on
            // S/D register encodings).
            if (!s.result.has_value()) {
                return {false, LowerError::DanglingRef, "XmmFromGpr without result"};
            }
            arm64::Reg rn;
            if (!reg_of(op.value, rn)) {
                return {false, LowerError::DanglingRef, "XmmFromGpr.value"};
            }
            Emitter::FpReg rd;
            if (!allocate_fp_scratch(*s.result, rd)) {
                return {false, LowerError::OutOfScratchRegs, "XmmFromGpr"};
            }
            const ir::FpSize fp_sz =
                op.size == ir::OpSize::I32 ? ir::FpSize::F32 : ir::FpSize::F64;
            emitter_.fmov_v_from_x(rd, rn, fp_sz);
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::GprFromXmm>) {
            // F2-IR-008. fmov w/x_rd, s/d_rn — copies V_rn low lane to
            // GPR with zero-extension when size is I32.
            if (!s.result.has_value()) {
                return {false, LowerError::DanglingRef, "GprFromXmm without result"};
            }
            Emitter::FpReg rn;
            if (!fp_reg_of(op.value, rn)) {
                return {false, LowerError::DanglingRef, "GprFromXmm.value"};
            }
            arm64::Reg rd;
            if (!allocate_scratch(*s.result, rd)) {
                return {false, LowerError::OutOfScratchRegs, "GprFromXmm"};
            }
            const ir::FpSize fp_sz =
                op.size == ir::OpSize::I32 ? ir::FpSize::F32 : ir::FpSize::F64;
            emitter_.fmov_x_from_v(rd, rn, fp_sz);
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::LoadVec>) {
            // F2-IR-007. 128-bit aligned/unaligned load from guest mem.
            // ARM64 `ldr Q, [Xn]` accepts both — alignment fault only on
            // strict-alignment systems we don't target.
            if (!s.result.has_value()) {
                return {false, LowerError::DanglingRef, "LoadVec without result"};
            }
            arm64::Reg raddr;
            if (!reg_of(op.addr, raddr)) {
                return {false, LowerError::DanglingRef, "LoadVec.addr"};
            }
            Emitter::FpReg rd;
            if (!allocate_fp_scratch(*s.result, rd)) {
                return {false, LowerError::OutOfScratchRegs, "LoadVec"};
            }
            emitter_.vld1_q(rd, raddr);
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::StoreVec>) {
            arm64::Reg raddr;
            if (!reg_of(op.addr, raddr)) {
                return {false, LowerError::DanglingRef, "StoreVec.addr"};
            }
            Emitter::FpReg rv;
            if (!fp_reg_of(op.value, rv)) {
                return {false, LowerError::DanglingRef, "StoreVec.value"};
            }
            emitter_.vst1_q(rv, raddr);
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::VecFpScalarBinOp>) {
            // F2-IR-006. ADDSS/ADDSD style: low lane = scalar op,
            // upper lanes preserved from lhs.
            if (!s.result.has_value()) {
                return {false, LowerError::DanglingRef,
                        "VecFpScalarBinOp requires a result ref"};
            }
            Emitter::FpReg rl, rr;
            if (!fp_reg_of(op.lhs, rl)) {
                return {false, LowerError::DanglingRef, "VecFpScalarBinOp.lhs"};
            }
            if (!fp_reg_of(op.rhs, rr)) {
                return {false, LowerError::DanglingRef, "VecFpScalarBinOp.rhs"};
            }
            Emitter::FpReg rd;
            if (!allocate_fp_scratch(*s.result, rd)) {
                return {false, LowerError::OutOfScratchRegs, "VecFpScalarBinOp"};
            }
            switch (op.op) {
                case ir::VecFpBinOpKind::Add: emitter_.vfadd_scalar(rd, rl, rr, op.size); break;
                case ir::VecFpBinOpKind::Sub: emitter_.vfsub_scalar(rd, rl, rr, op.size); break;
                case ir::VecFpBinOpKind::Mul: emitter_.vfmul_scalar(rd, rl, rr, op.size); break;
                case ir::VecFpBinOpKind::Div: emitter_.vfdiv_scalar(rd, rl, rr, op.size); break;
            }
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::FpConstant>) {
            // F1-BK-013. Materialise an FP constant in a scratch V reg.
            if (!s.result.has_value()) {
                return {false, LowerError::DanglingRef,
                        "FpConstant requires a result ref"};
            }
            Emitter::FpReg rd;
            if (!allocate_fp_scratch(*s.result, rd)) {
                return {false, LowerError::OutOfScratchRegs, "FpConstant"};
            }
            emitter_.fmov_imm(rd, op.bits, op.size);
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::FpBinOp>) {
            // F1-BK-013. fadd / fsub / fmul / fdiv on a freshly-allocated
            // V scratch.
            if (!s.result.has_value()) {
                return {false, LowerError::DanglingRef,
                        "FpBinOp requires a result ref"};
            }
            Emitter::FpReg rl, rr;
            if (!fp_reg_of(op.lhs, rl)) {
                return {false, LowerError::DanglingRef, "FpBinOp.lhs"};
            }
            if (!fp_reg_of(op.rhs, rr)) {
                return {false, LowerError::DanglingRef, "FpBinOp.rhs"};
            }
            Emitter::FpReg rd;
            if (!allocate_fp_scratch(*s.result, rd)) {
                return {false, LowerError::OutOfScratchRegs, "FpBinOp"};
            }
            switch (op.op) {
                case ir::FpBinOpKind::Add: emitter_.fadd(rd, rl, rr, op.size); break;
                case ir::FpBinOpKind::Sub: emitter_.fsub(rd, rl, rr, op.size); break;
                case ir::FpBinOpKind::Mul: emitter_.fmul(rd, rl, rr, op.size); break;
                case ir::FpBinOpKind::Div: emitter_.fdiv(rd, rl, rr, op.size); break;
            }
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::Fence>) {
            // F1-BK-023. Map x86 fences to ARM64 DMB ISH variants.
            switch (op.kind) {
                case ir::FenceKind::Mfence:
                    emitter_.dmb(Emitter::BarrierKind::Ish);   break;
                case ir::FenceKind::Lfence:
                    emitter_.dmb(Emitter::BarrierKind::IshLd); break;
                case ir::FenceKind::Sfence:
                    emitter_.dmb(Emitter::BarrierKind::IshSt); break;
            }
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::CallRel>
                        || std::is_same_v<T, ir::CallReg>
                        || std::is_same_v<T, ir::RetAdjusted>) {
            // Placeholder terminator until the dispatcher learns about
            // guest call/ret. For now: park the (next) target PC in x0
            // and ret to the dispatcher; it'll loop back in for the
            // next translation. Stack management for RetAdjusted's
            // pop_bytes is deferred to F1-RT-008 follow-up.
            std::uint64_t next_pc = 0;
            if constexpr (std::is_same_v<T, ir::CallRel>) {
                next_pc = op.target_guest_pc;
            } else if constexpr (std::is_same_v<T, ir::CallReg>) {
                arm64::Reg rt;
                if (!reg_of(op.target, rt)) {
                    return {false, LowerError::DanglingRef, "CallReg.target"};
                }
                emitter_.mov_reg_reg(arm64::Reg::X0, rt);
                if (options_.emit_ret_on_terminator) emitter_.ret();
                return {};
            }
            // CallRel / RetAdjusted both fall through with next_pc above.
            emitter_.mov_imm64(arm64::Reg::X0, next_pc);
            if (options_.emit_ret_on_terminator) emitter_.ret();
            return {};
        }
        else {
            // Compare, LoadMem, StoreMem, LoadMemTSO, StoreMemTSO are
            // already lowered above. Anything reaching here is genuinely
            // unsupported.
            return {false, LowerError::UnsupportedOp, "op not yet lowered"};
        }
    }, s.op);
}

}  // namespace prisma::backend
