// prisma/cpu_state.hpp — guest CPU state frame.
//
// Layout of the guest x86_64 CPU state the runtime passes between
// translated blocks. The Fase 0 MVP held only the 16 general-purpose
// registers; F1-RT-012 grows the frame with the SSE/AVX vector
// register file (XMM0..XMM15 — 256-bit YMM coverage is a future
// extension via the same offset table) and a placeholder x87 stack.
//
// Access from generated ARM64 code goes through a pinned register
// holding the frame pointer; backend::abi::emit_block_prologue /
// emit_block_epilogue_and_ret keep host registers in sync with this
// frame on every dispatcher round trip.

#pragma once

#include <array>
#include <cstddef>
#include <cstdint>

#include "prisma/ir.hpp"  // ir::Gpr, ir::kGprCount

namespace prisma::runtime {

// XMM (SSE) register count. AVX-256 doubles the size by widening
// each lane to 32 bytes (YMM); we'll grow `xmm[]` to a 32-byte
// element type when the AVX path lands.
inline constexpr std::size_t kXmmCount = 16;

// 128-bit guest XMM register, naturally aligned. Two 64-bit lanes
// are the canonical access pattern for SSE2 integer code; floating-
// point ops view the same bits as packed F32 / F64.
struct alignas(16) XmmReg {
    std::uint64_t lo{0};
    std::uint64_t hi{0};
};

// x87 8-deep stack of 80-bit floats. We model each slot as a 16-byte
// container so the layout stays naturally aligned and we leave room
// for the tag word the FXSAVE / FNINIT semantics need. The full
// MMX/x87 state is the next-larger problem; this is the placeholder
// the runtime uses today so PUSH/POP of the host's V regs has a
// stable spill location.
struct alignas(16) X87Slot {
    std::uint64_t lo{0};   // mantissa low
    std::uint64_t hi{0};   // mantissa high + sign + exp (10 bytes used)
};

inline constexpr std::size_t kX87StackDepth = 8;

// Matches x86 register-encoding order: rax, rcx, rdx, rbx, rsp, rbp,
// rsi, rdi, r8..r15. Index with `static_cast<std::size_t>(ir::Gpr::X)`.
struct CpuStateFrame {
    std::array<std::uint64_t, ir::kGprCount> gpr{};

    // The guest program counter. Updated by the dispatcher between
    // block executions.
    std::uint64_t guest_pc{0};

    // F1-RT-012: SSE register file. 16 × 128-bit XMM registers.
    std::array<XmmReg, kXmmCount> xmm{};

    // F1-RT-012: x87 / MMX stack. 8 entries, each 16 bytes (10 bytes
    // for the 80-bit float, 6 bytes reserved for tag/exception/etc.
    // when we wire FXSAVE semantics).
    std::array<X87Slot, kX87StackDepth> x87{};

    // x87 status word + control word + TOS pointer. 16 bits each;
    // packed into a u64 for now (low 16 = status, next 16 = control,
    // next 8 = top-of-stack, rest reserved).
    std::uint64_t x87_status_control{0};

    // MXCSR (SSE control / status). Default 0x1F80 (mask all
    // exceptions, FZ off, RC = nearest).
    std::uint32_t mxcsr{0x1F80u};

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

    // F1-RT-012 — byte offset to `frame.xmm[idx].lo`. The high lane
    // is at `+ 8` from this offset. The compiler pads `guest_pc`
    // (8 bytes at offset 128) up to xmm[]'s 16-byte alignment, so
    // xmm[] actually starts at offset 144.
    [[nodiscard]] static constexpr std::int32_t xmm_offset_bytes(std::size_t idx) noexcept {
        return 144 + static_cast<std::int32_t>(idx) * 16;
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
static_assert(offsetof(CpuStateFrame, guest_pc) == 16 * 8,
              "guest_pc must follow gpr[] immediately");
static_assert(offsetof(CpuStateFrame, xmm) == 144,
              "xmm[] starts at 144 (16 GPR×8 + guest_pc×8 + 8 pad for 16-align)");
static_assert(sizeof(XmmReg) == 16, "XmmReg is 128 bits");
static_assert(alignof(XmmReg) == 16, "XmmReg is naturally aligned");
static_assert(sizeof(X87Slot) == 16, "X87Slot is 16 bytes (10 used + 6 pad)");

}  // namespace prisma::runtime
