// core/src/runtime/dispatcher.cpp — implementation.

#include "prisma/dispatcher.hpp"

#include <unordered_set>
#include <utility>

namespace prisma::runtime {

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
    // the slot RSP currently points at, which is 0 → halt.
    state_.gpr[static_cast<std::size_t>(ir::Gpr::Rsp)] =
        reinterpret_cast<std::uint64_t>(
            &halt_return_stack_[kHaltReturnStackSlots - 1]);
}

DispatchResult Dispatcher::run(std::uint64_t entry_pc,
                               std::size_t max_steps) {
    DispatchStats stats;
    std::unordered_set<std::uint64_t> seen_pcs;

    std::uint64_t pc = entry_pc;
    state_.guest_pc = pc;

    for (std::size_t step = 0; step < max_steps; ++step) {
        // Halt-before-execute so the caller can configure halt PCs that
        // include the entry (useful for tests that check "we never
        // enter this region").
        if (halt_pcs_.count(pc) != 0) {
            return {DispatchExit::Halted, pc, stats, {}};
        }

        const auto bytes = reader_(pc);
        if (bytes.empty()) {
            return {DispatchExit::FetchFailed, pc, stats,
                    "guest memory reader returned no bytes at the current PC"};
        }

        auto r = translator_.translate(pc, bytes);
        if (std::holds_alternative<translator::TranslateError>(r)) {
            return {DispatchExit::TranslationFailed, pc, stats,
                    "translator rejected the guest region"};
        }
        const auto& block = std::get<translator::TranslatedBlock>(r);

        // Invoke the block. Calling convention established by the
        // Translator's prologue/epilogue:
        //   input:  x0 = CpuStateFrame*  (we pass &state_)
        //   output: x0 = next guest PC
        // The block's prologue loads guest GPRs from *state_ into
        // pinned host regs, runs the body, and the epilogue stores
        // them back — so guest state survives across blocks.
        using Fn = std::uint64_t (*)(CpuStateFrame*);
        Fn fn = reinterpret_cast<Fn>(const_cast<std::uint8_t*>(block.code_entry));
        const std::uint64_t next_pc = fn(&state_);

        ++stats.blocks_executed;
        ++stats.steps_taken;
        seen_pcs.insert(pc);

        pc = next_pc;
        state_.guest_pc = pc;
    }

    stats.unique_pcs_seen = seen_pcs.size();
    return {DispatchExit::StepLimit, pc, stats, {}};
}

}  // namespace prisma::runtime
