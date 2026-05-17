// core/src/runtime/dispatcher.cpp - implementation.

#include "prisma/dispatcher.hpp"

#include <unordered_set>
#include <utility>
#include <variant>

namespace prisma::runtime {

namespace {

using Fn = std::uint64_t (*)(CpuStateFrame*);

std::uint64_t execute_block(const translator::TranslatedBlock& block,
                            CpuStateFrame& state) {
    Fn fn = reinterpret_cast<Fn>(const_cast<std::uint8_t*>(block.code_entry));
    return fn(&state);
}

bool direct_thread_candidate(const translator::TranslatedBlock& block,
                             std::uint64_t next_pc) {
    if (block.exit_kind == translator::BlockExitKind::JumpRel) {
        return next_pc == block.target_guest_pc;
    }
    if (block.exit_kind == translator::BlockExitKind::CondJumpRel) {
        return next_pc == block.target_guest_pc
            || next_pc == block.fallthrough_guest_pc;
    }
    return false;
}

}  // namespace

Dispatcher::Dispatcher(translator::Translator& t, GuestMemoryReader r)
    : translator_(t), reader_(std::move(r)) {
    halt_pcs_.insert(CpuStateFrame::kHaltSentinel);
}

void Dispatcher::add_halt_pc(std::uint64_t pc) {
    halt_pcs_.insert(pc);
}

void Dispatcher::install_halt_return_stack() {
    for (auto& slot : halt_return_stack_) slot = 0;
    // Point RSP at the top slot. The first push (RSP -= 8) lands at
    // the next slot down; the outermost RET (no preceding push) pops
    // the slot RSP currently points at, which is 0 -> halt.
    state_.gpr[static_cast<std::size_t>(ir::Gpr::Rsp)] =
        reinterpret_cast<std::uint64_t>(
            &halt_return_stack_[kHaltReturnStackSlots - 1]);
}

DispatchResult Dispatcher::run(std::uint64_t entry_pc,
                               std::size_t max_steps) {
    DispatchStats stats;
    std::unordered_set<std::uint64_t> seen_pcs;
    return_stack_depth_ = 0;

    std::uint64_t pc = entry_pc;
    state_.guest_pc = pc;

    std::size_t step = 0;
    while (step < max_steps) {
        // Halt-before-execute so the caller can configure halt PCs that
        // include the entry.
        if (halt_pcs_.count(pc) != 0) {
            stats.unique_pcs_seen = seen_pcs.size();
            return {DispatchExit::Halted, pc, stats, {}};
        }

        const auto bytes = reader_(pc);
        if (bytes.empty()) {
            stats.unique_pcs_seen = seen_pcs.size();
            return {DispatchExit::FetchFailed, pc, stats,
                    "guest memory reader returned no bytes at the current PC"};
        }

        auto r = translator_.translate(pc, bytes);
        if (std::holds_alternative<translator::TranslateError>(r)) {
            stats.unique_pcs_seen = seen_pcs.size();
            return {DispatchExit::TranslationFailed, pc, stats,
                    "translator rejected the guest region"};
        }
        translator::TranslatedBlock block =
            std::get<translator::TranslatedBlock>(r);

        while (true) {
            const std::uint64_t next_pc = execute_block(block, state_);

            if (block.exit_kind == translator::BlockExitKind::CallRel
                || block.exit_kind == translator::BlockExitKind::CallReg) {
                ++stats.ras_pushes;
                if (return_stack_depth_ == return_stack_.size()) {
                    return_stack_[return_stack_.size() - 1] = block.return_guest_pc;
                    ++stats.ras_overflows;
                } else {
                    return_stack_[return_stack_depth_++] = block.return_guest_pc;
                }
            } else if (block.exit_kind == translator::BlockExitKind::RetAdjusted) {
                ++stats.ras_pops;
                if (return_stack_depth_ == 0) {
                    ++stats.ras_underflows;
                } else {
                    const std::uint64_t predicted =
                        return_stack_[--return_stack_depth_];
                    if (predicted == next_pc) {
                        ++stats.ras_hits;
                    } else {
                        ++stats.ras_misses;
                    }
                }
            }

            ++stats.blocks_executed;
            ++stats.steps_taken;
            seen_pcs.insert(pc);
            ++step;

            const bool can_thread = direct_thread_candidate(block, next_pc);
            pc = next_pc;
            state_.guest_pc = pc;

            if (step >= max_steps || !can_thread || halt_pcs_.count(pc) != 0) {
                break;
            }

            const auto next_bytes = reader_(pc);
            if (next_bytes.empty()) {
                stats.unique_pcs_seen = seen_pcs.size();
                return {DispatchExit::FetchFailed, pc, stats,
                        "guest memory reader returned no bytes at the current PC"};
            }

            auto cached = translator_.lookup_cached(pc, next_bytes);
            if (!cached) {
                ++stats.direct_thread_misses;
                break;
            }

            ++stats.direct_thread_hits;
            block = *cached;
        }
    }

    stats.unique_pcs_seen = seen_pcs.size();
    return {DispatchExit::StepLimit, pc, stats, {}};
}

}  // namespace prisma::runtime
