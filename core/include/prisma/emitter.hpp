// prisma/emitter.hpp — ARM64 code emitter built on vixl.
//
// This is the thin Prisma-facing API over vixl's MacroAssembler. Day 1 it
// exposes just enough to lower our 12-opcode IR MVP to bytes. As the IR
// grows (Fase 1-2), this API grows along with it.
//
// Invariants:
//   * One `Emitter` per translation unit (basic block or function). Not
//     thread-safe — each thread must own its Emitter.
//   * `finalize()` must be called before reading `code_bytes()`. After
//     finalize the Emitter is read-only.
//   * All emitted code targets 64-bit ARM64. 32-bit W-regs and SIMD come
//     later.

#pragma once

#include <cstddef>
#include <cstdint>
#include <span>
#include <vector>

#include "prisma/arm64_encoding.hpp"  // for Reg enum & host_reg_for

namespace prisma::backend {

class Emitter {
public:
    Emitter();
    ~Emitter();

    Emitter(const Emitter&) = delete;
    Emitter& operator=(const Emitter&) = delete;
    Emitter(Emitter&&) = delete;
    Emitter& operator=(Emitter&&) = delete;

    // --- instruction-level API (grows as IR grows) ---

    // movz Xd, #imm16, lsl #(hw*16)
    void movz(arm64::Reg rd, std::uint16_t imm16, unsigned hw = 0);

    // mov Xd, #imm64 (may emit up to 4 instructions: movz + movk*)
    void mov_imm64(arm64::Reg rd, std::uint64_t imm);

    // ret xN  (default x30)
    void ret(arm64::Reg rn = arm64::Reg::X30);

    // --- lifecycle ---

    // Finalize the buffer: resolve labels, emit any literal pool, flush
    // internal state. Call exactly once before reading bytes.
    void finalize();

    // Raw code bytes. Only valid after finalize().
    [[nodiscard]] std::span<const std::uint8_t> code_bytes() const noexcept;

    // Disassemble the emitted code (after finalize). One instruction per
    // line. Primarily for debugging / test failure messages.
    [[nodiscard]] std::string disassemble() const;

private:
    struct Impl;             // pimpl — hides vixl headers from our API
    Impl* impl_;
};

}  // namespace prisma::backend
