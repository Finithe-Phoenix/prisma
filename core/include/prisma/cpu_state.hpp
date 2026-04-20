// prisma/cpu_state.hpp — guest CPU state frame.
//
// Layout of the guest x86_64 CPU state the runtime passes between
// translated blocks. For Fase 0 MVP this holds only the 16 general-
// purpose registers; flags / SIMD / x87 land with F1-IR-003 (flags)
// and F2-IR-001..003 (SIMD).
//
// Access from generated ARM64 code goes through a pinned register
// (not implemented yet — in Fase 1 the Lowerer will gain awareness
// of a state pointer). Until then the dispatcher only reads/writes
// this struct from C++.

#pragma once

#include <array>
#include <cstddef>
#include <cstdint>

#include "prisma/ir.hpp"  // ir::Gpr, ir::kGprCount

namespace prisma::runtime {

// Matches x86 register-encoding order: rax, rcx, rdx, rbx, rsp, rbp,
// rsi, rdi, r8..r15. Index with `static_cast<std::size_t>(ir::Gpr::X)`.
struct CpuStateFrame {
    std::array<std::uint64_t, ir::kGprCount> gpr{};

    // The guest program counter. Updated by the dispatcher between
    // block executions.
    std::uint64_t guest_pc{0};

    // Halt sentinel — when a translated block returns this value in x0,
    // the dispatcher exits the run loop cleanly. The IR::Return lowerer
    // emits code that sets x0 = 0 before `ret`, so pc=0 is the default
    // halt.
    static constexpr std::uint64_t kHaltSentinel = 0;

    [[nodiscard]] std::uint64_t& operator[](ir::Gpr g) noexcept {
        return gpr[static_cast<std::size_t>(g)];
    }
    [[nodiscard]] std::uint64_t operator[](ir::Gpr g) const noexcept {
        return gpr[static_cast<std::size_t>(g)];
    }

    // Byte offset from `&frame` to `frame.gpr[idx]`. Generated code
    // uses these to emit `ldr x_pinned, [x19, #offset]` in the block
    // prologue / epilogue. See `translator.cpp`.
    [[nodiscard]] static constexpr std::int32_t gpr_offset_bytes(ir::Gpr g) noexcept {
        return static_cast<std::int32_t>(static_cast<std::size_t>(g)) * 8;
    }
};

// Guarantees the C++ struct layout matches what the Translator emits
// into ARM64 code. If any of these fail, regenerate the pinned offsets
// used in translator.cpp's prologue/epilogue.
static_assert(std::is_standard_layout_v<CpuStateFrame>,
              "CpuStateFrame must be standard-layout so offsetof is defined");
static_assert(offsetof(CpuStateFrame, gpr) == 0,
              "gpr[] must be the first field; prologue uses offsets from the frame base");
static_assert(sizeof(CpuStateFrame::gpr) == ir::kGprCount * sizeof(std::uint64_t),
              "gpr[] must be a tight array of 16 uint64_t");

}  // namespace prisma::runtime
