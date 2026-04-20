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

        // Invoke the block. AAPCS64: the caller sets x30 (LR) via blr,
        // the block returns to LR via ret, and the integer return value
        // sits in x0 — which by our contract is the next guest PC.
        using Fn = std::uint64_t (*)();
        Fn fn = reinterpret_cast<Fn>(const_cast<std::uint8_t*>(block.code_entry));
        const std::uint64_t next_pc = fn();

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
