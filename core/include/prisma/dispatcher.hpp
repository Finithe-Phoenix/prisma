// prisma/dispatcher.hpp — run loop that chains translated blocks.
//
// The dispatcher closes the last big gap between "we can translate a
// block" and "we can run a program": given an entry PC and a way to
// read guest memory, it alternates translate + execute until the
// program halts, hits a step limit, or fails.
//
// Execution model (MVP):
//   1. Each translated block's body ends with ARM64 `ret`, placing the
//      next guest PC in x0 before returning. This is the convention the
//      Lowerer enforces:
//        * IR::Return      → mov x0, #0 ; ret
//          (0 is the halt sentinel, CpuStateFrame::kHaltSentinel.)
//        * IR::JumpRel     → mov x0, <target> ; ret
//        * IR::CondJumpRel → csel + ret (x0 is taken or fallthrough).
//        * IR::Call*       → push guest return PC, x0 = callee.
//        * IR::RetAdjusted → pop guest return PC, x0 = popped target.
//
//   2. The dispatcher invokes the block via a function pointer cast of
//      the code_entry; `blr` (AArch64 calling convention) sets the link
//      register, the block's `ret` returns, and the function-call return
//      value is whatever was in x0. That's the next guest PC.
//
//   3. Cross-block guest state persists through CpuStateFrame. The
//      translator prologue loads pinned guest registers from the frame;
//      its epilogue writes them back before returning to the dispatcher.

#pragma once

#include <array>
#include <cstddef>
#include <cstdint>
#include <functional>
#include <optional>
#include <span>
#include <string>
#include <unordered_set>
#include <variant>

#include "prisma/cpu_state.hpp"
#include "prisma/translator.hpp"

namespace prisma::runtime {

// The dispatcher asks for the bytes at a guest PC on demand. Returning
// an empty span signals "no memory here" — the dispatcher treats that
// as a fetch fault and stops.
using GuestMemoryReader =
    std::function<std::span<const std::uint8_t>(std::uint64_t pc)>;

enum class DispatchExit {
    Halted,              // block returned the halt sentinel (e.g. 0).
    StepLimit,           // ran out of max_steps.
    FetchFailed,         // GuestMemoryReader returned empty span.
    TranslationFailed,   // Translator returned an error.
};

struct DispatchStats {
    std::size_t blocks_executed{0};
    std::size_t steps_taken{0};
    std::size_t unique_pcs_seen{0};
    std::size_t ras_pushes{0};
    std::size_t ras_pops{0};
    std::size_t ras_hits{0};
    std::size_t ras_misses{0};
    std::size_t ras_overflows{0};
    std::size_t ras_underflows{0};
    std::size_t direct_thread_hits{0};
    std::size_t direct_thread_misses{0};
    std::size_t direct_thread_installs{0};
    std::size_t direct_jit_patch_attempts{0};
    std::size_t direct_jit_patch_applied{0};
    std::size_t direct_jit_patch_rejected{0};
    std::size_t direct_jit_patch_unpatches{0};
    std::size_t direct_jit_patch_executes{0};
};

struct DispatchResult {
    DispatchExit exit;
    std::uint64_t final_pc;
    DispatchStats stats;
    std::string message;  // human-readable context on errors
};

class Dispatcher {
public:
    Dispatcher(translator::Translator& translator,
               GuestMemoryReader reader);

    // Add another PC value that the dispatcher treats as "halted
    // cleanly" (in addition to `CpuStateFrame::kHaltSentinel` which is
    // always present). Useful for tests that want to stop at a known
    // boundary.
    void add_halt_pc(std::uint64_t pc);

    // Run starting at `entry_pc` until one of the exit conditions
    // triggers. Never runs more than `max_steps` blocks. The CpuState
    // frame the dispatcher owns is available via `state()` for before/
    // after inspection; MVP does not auto-propagate it into generated
    // code.
    [[nodiscard]] DispatchResult run(std::uint64_t entry_pc,
                                     std::size_t max_steps);

    [[nodiscard]] CpuStateFrame& state() noexcept { return state_; }
    [[nodiscard]] const CpuStateFrame& state() const noexcept { return state_; }

    // Helper for tests that opt into real CALL/RET semantics on the
    // translator (`translator::Translator::set_real_call_ret(true)`).
    // Allocates a small internal halt-return stack (16 × u64 = 128 B,
    // zero-initialised) and points `state.gpr[Rsp]` at the top slot.
    //
    // The bottom slot is `0` (= `CpuStateFrame::kHaltSentinel`), so
    // the *outermost* RET in a test program pops 0 and the dispatcher
    // halts cleanly. Inner CALL/RET pairs work normally: CALL pushes
    // (RSP -= 8 → an interior slot), the callee's RET pops back.
    //
    // Safe to call multiple times; each call resets the stack to all
    // zeroes and re-points Rsp at the top.
    void install_halt_return_stack();

private:
    translator::Translator& translator_;
    GuestMemoryReader reader_;
    std::unordered_set<std::uint64_t> halt_pcs_;
    CpuStateFrame state_;
    static constexpr std::size_t kReturnStackSlots = 32;
    std::array<std::uint64_t, kReturnStackSlots> return_stack_{};
    std::size_t return_stack_depth_{0};
    static constexpr std::size_t kHaltReturnStackSlots = 16;
    alignas(8) std::uint64_t halt_return_stack_[kHaltReturnStackSlots]{};
};

}  // namespace prisma::runtime
