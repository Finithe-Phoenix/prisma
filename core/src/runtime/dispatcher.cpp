// core/src/runtime/dispatcher.cpp - implementation.

#include "prisma/dispatcher.hpp"

#include <optional>
#include <unordered_set>
#include <utility>
#include <variant>

#include "prisma/signal_handler.hpp"

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
    if (block.exit_kind == translator::BlockExitKind::CallRel) {
        return next_pc == block.target_guest_pc;
    }
    if (block.exit_kind == translator::BlockExitKind::RepStos
        || block.exit_kind == translator::BlockExitKind::RepMovs) {
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
    auto account_executed =
        [&](std::uint64_t executed_pc,
            const translator::TranslatedBlock& executed_block,
            std::uint64_t observed_next_pc) {
            if (executed_block.exit_kind == translator::BlockExitKind::CallRel
                || executed_block.exit_kind == translator::BlockExitKind::CallReg) {
                ++stats.ras_pushes;
                if (return_stack_depth_ == return_stack_.size()) {
                    return_stack_[return_stack_.size() - 1] =
                        executed_block.return_guest_pc;
                    ++stats.ras_overflows;
                } else {
                    return_stack_[return_stack_depth_++] =
                        executed_block.return_guest_pc;
                }
            } else if (executed_block.exit_kind
                       == translator::BlockExitKind::RetAdjusted) {
                ++stats.ras_pops;
                if (return_stack_depth_ == 0) {
                    ++stats.ras_underflows;
                } else {
                    const std::uint64_t predicted =
                        return_stack_[--return_stack_depth_];
                    if (predicted == observed_next_pc) {
                        ++stats.ras_hits;
                    } else {
                        ++stats.ras_misses;
                    }
                }
            }

            ++stats.blocks_executed;
            ++stats.steps_taken;
            seen_pcs.insert(executed_pc);
            ++step;
        };
    auto try_patch_direct_exit =
        [&](std::uint64_t source_pc, std::uint64_t target_pc) {
            ++stats.direct_jit_patch_attempts;
            const bool was_patched =
                translator_.direct_exit_is_patched(source_pc);
            const auto result =
                translator_.patch_direct_exit(source_pc, target_pc);
            if (result == translator::DirectPatchResult::Ok) {
                if (!was_patched
                    && translator_.direct_exit_is_patched(source_pc)) {
                    ++stats.direct_jit_patch_applied;
                }
            } else {
                ++stats.direct_jit_patch_rejected;
            }
        };

    while (step < max_steps) {
        // Drain SMC fault bookkeeping queued by the SIGSEGV handler
        // (the handler itself is async-signal-safe and only
        // tombstones; the invalidation callbacks run here, in normal
        // context). Near-free when nothing is pending.
        (void)drain_smc_invalidations();

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
            const std::uint64_t source_pc = pc;
            std::uint64_t patched_target_pc = 0;
            std::optional<translator::TranslatedBlock> patched_target =
                translator_.active_direct_patch_target(source_pc);
            if (patched_target) {
                patched_target_pc = block.direct_patch.target_guest_pc;
                bool keep_patch =
                    (max_steps - step >= 2)
                    && halt_pcs_.count(patched_target_pc) == 0;
                if (keep_patch) {
                    const auto target_bytes = reader_(patched_target_pc);
                    if (target_bytes.empty()) {
                        keep_patch = false;
                    } else {
                        auto verified =
                            translator_.lookup_cached(patched_target_pc,
                                                      target_bytes);
                        keep_patch = verified.has_value()
                            && verified->code_entry == patched_target->code_entry;
                        if (keep_patch) {
                            patched_target = *verified;
                        }
                    }
                }
                if (!keep_patch) {
                    if (translator_.unpatch_direct_exit(source_pc)
                        != translator::DirectPatchResult::Ok) {
                        stats.unique_pcs_seen = seen_pcs.size();
                        return {DispatchExit::TranslationFailed, source_pc, stats,
                                "failed to unpatch a direct JIT branch"};
                    }
                    ++stats.direct_jit_patch_unpatches;
                    patched_target.reset();
                }
            }

            const std::uint64_t next_pc = execute_block(block, state_);

            const translator::TranslatedBlock* threading_block = &block;
            std::uint64_t threading_pc = source_pc;
            if (patched_target) {
                ++stats.direct_jit_patch_executes;
                account_executed(source_pc, block, patched_target_pc);
                account_executed(patched_target_pc, *patched_target, next_pc);
                threading_block = &*patched_target;
                threading_pc = patched_target_pc;
            } else {
                account_executed(source_pc, block, next_pc);
            }

            const bool can_thread =
                direct_thread_candidate(*threading_block, next_pc);
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
                auto installed = translator_.translate(pc, next_bytes);
                if (std::holds_alternative<translator::TranslateError>(installed)) {
                    stats.unique_pcs_seen = seen_pcs.size();
                    return {DispatchExit::TranslationFailed, pc, stats,
                            "translator rejected the guest region"};
                }
                auto successor = std::get<translator::TranslatedBlock>(installed);
                if (successor.from_cache) {
                    ++stats.direct_thread_hits;
                } else {
                    ++stats.direct_thread_installs;
                }
                if (threading_block->direct_patch.available
                    && threading_block->direct_patch.auto_patch_safe
                    && threading_block->direct_patch.target_guest_pc == pc
                    && halt_pcs_.count(pc) == 0) {
                    try_patch_direct_exit(threading_pc, pc);
                }
                block = successor;
                continue;
            }

            ++stats.direct_thread_hits;
            if (threading_block->direct_patch.available
                && threading_block->direct_patch.auto_patch_safe
                && threading_block->direct_patch.target_guest_pc == pc
                && halt_pcs_.count(pc) == 0) {
                try_patch_direct_exit(threading_pc, pc);
            }
            block = *cached;
        }
    }

    stats.unique_pcs_seen = seen_pcs.size();
    if (halt_pcs_.count(pc) != 0) {
        return {DispatchExit::Halted, pc, stats, {}};
    }
    return {DispatchExit::StepLimit, pc, stats, {}};
}

}  // namespace prisma::runtime
