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
            else if constexpr (std::is_same_v<T, ir::Extend>)      { bump(op.value, i); }
            else if constexpr (std::is_same_v<T, ir::Truncate>)    { bump(op.value, i); }
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
            // Constant, LoadReg, LoadSegBase, Jump, JumpRel, CallRel,
            // RetAdjusted, Cpuid, Syscall, Trap, Fence, CondJumpRel, Return have
            // no operand refs — nothing to bump.
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
            // writes cannot clobber this value. Narrow loads become
            // canonical zero-extended SSA values.
            emitter_.mov_reg_reg(rd, arm64::host_reg_for(op.reg), op.size);
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::StoreReg>) {
            arm64::Reg src;
            if (!reg_of(op.value, src)) {
                return {false, LowerError::DanglingRef, "StoreReg.value"};
            }
            emitter_.store_reg_reg(arm64::host_reg_for(op.reg), src, op.size);
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
        else if constexpr (std::is_same_v<T, ir::Extend>) {
            if (!s.result) return {false, LowerError::DanglingRef, "Extend without result ref"};
            arm64::Reg rn;
            if (!reg_of(op.value, rn)) return {false, LowerError::DanglingRef, "Extend.value"};
            arm64::Reg rd;
            if (!allocate_scratch(*s.result, rd)) {
                return {false, LowerError::OutOfScratchRegs, "Extend"};
            }

            if (!op.is_signed) {
                emitter_.zero_extend(rd, rn, op.from_size);
                if (ir::bit_width(op.to_size) < ir::bit_width(op.from_size)) {
                    emitter_.truncate(rd, rd, op.to_size);
                }
                return {};
            }

            if (op.to_size == ir::OpSize::I64
                || ir::bit_width(op.to_size) <= ir::bit_width(op.from_size)) {
                emitter_.sign_extend(rd, rn, op.from_size);
                if (op.to_size != ir::OpSize::I64) {
                    emitter_.truncate(rd, rd, op.to_size);
                }
                return {};
            }

            arm64::Reg tmp;
            if (!allocate_temporary(tmp)) {
                return {false, LowerError::OutOfScratchRegs, "Extend temporary"};
            }
            emitter_.sign_extend(tmp, rn, op.from_size);
            emitter_.truncate(rd, tmp, op.to_size);
            return {};
        }
        else if constexpr (std::is_same_v<T, ir::Truncate>) {
            if (!s.result) return {false, LowerError::DanglingRef, "Truncate without result ref"};
            arm64::Reg rn;
            if (!reg_of(op.value, rn)) return {false, LowerError::DanglingRef, "Truncate.value"};
            arm64::Reg rd;
            if (!allocate_scratch(*s.result, rd)) {
                return {false, LowerError::OutOfScratchRegs, "Truncate"};
            }
            emitter_.truncate(rd, rn, op.to_size);
            return {};
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
        else if constexpr (std::is_same_v<T, ir::Fence>) {
            emitter_.fence(op.kind);
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
        else {
            // Compare, Jump, CondJump, LoadMem, StoreMem, LoadMemTSO,
            // StoreMemTSO — deferred to future sessions.
            return {false, LowerError::UnsupportedOp, "op not yet lowered"};
        }
    }, s.op);
}

}  // namespace prisma::backend
