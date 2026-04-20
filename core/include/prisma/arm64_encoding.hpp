// prisma/arm64_encoding.hpp — hand-rolled ARM64 encoders for a narrow set
// of instructions Prisma will generate directly before vixl integration.
//
// Purpose of this file: validate our ISA understanding. Every encoder here
// has a test that compares against ARM Architecture Reference Manual bit
// layouts. When vixl integration lands (planned RFC 0002), most of these
// helpers become vestigial — but they will remain as a compile-time-tested
// ground truth we can diff vixl output against.

#pragma once

#include <cstdint>

#include "prisma/ir.hpp"

namespace prisma::arm64 {

// ARM64 registers are 5-bit indices. 0-30 are GPR, 31 is SP or XZR/WZR
// depending on context. Prisma never emits SP/XZR references by number —
// callers must use explicit helpers.
enum class Reg : std::uint8_t {
    X0  = 0,  X1,  X2,  X3,  X4,  X5,  X6,  X7,
    X8  = 8,  X9,  X10, X11, X12, X13, X14, X15,
    X16 = 16, X17, X18, X19, X20, X21, X22, X23,
    X24 = 24, X25, X26, X27, X28, X29, X30,
    // 31 is either SP or XZR; we do not name it here to force deliberate use.
};

// An encoded 32-bit ARM64 instruction. Kept as a strong type so we do not
// accidentally mix instructions with arbitrary uint32_t.
struct Instr {
    std::uint32_t raw;
    [[nodiscard]] constexpr bool operator==(const Instr& o) const noexcept {
        return raw == o.raw;
    }
};

// ---------------------------------------------------------------------------
// MOVZ / MOVK / MOVN — wide immediate moves.
// ---------------------------------------------------------------------------
//
// ARM ARM C6.2.194–C6.2.196. Encoding:
//   sf(1) | opc(2) | 100101 | hw(2) | imm16(16) | Rd(5)
//
//   sf  = 0 → 32-bit variant (W register), 1 → 64-bit (X register)
//   opc = 00 MOVN, 10 MOVZ, 11 MOVK
//   hw  = shift amount / 16 (0..3 for X regs, 0..1 for W regs)
//
// We expose MOVZ (most common for constant loads) first. MOVK/MOVN follow
// when we need to build larger constants across multiple moves.

[[nodiscard]] constexpr Instr movz_x(Reg rd, std::uint16_t imm16, unsigned hw) noexcept {
    // assert(hw <= 3) would be ideal; we omit until we pick an assertion policy.
    std::uint32_t sf    = 1u;                         // 64-bit
    std::uint32_t opc   = 0b10u;                      // MOVZ
    std::uint32_t fixed = 0b100101u;                  // fixed pattern
    std::uint32_t rd_v  = static_cast<std::uint32_t>(rd) & 0x1Fu;
    std::uint32_t hw_v  = hw & 0x3u;
    std::uint32_t imm   = static_cast<std::uint32_t>(imm16);
    std::uint32_t raw =
          (sf    << 31)
        | (opc   << 29)
        | (fixed << 23)
        | (hw_v  << 21)
        | (imm   << 5)
        | (rd_v  << 0);
    return Instr{raw};
}

// ---------------------------------------------------------------------------
// RET — return from subroutine.
// ---------------------------------------------------------------------------
//
// ARM ARM C6.2.279. Encoding:
//   1101 0110 0101 1111 0000 00 | Rn(5) | 0 0000
//
//   Rn is typically X30 (LR). RET without an explicit register defaults to X30.

[[nodiscard]] constexpr Instr ret(Reg rn = Reg::X30) noexcept {
    std::uint32_t rn_v = static_cast<std::uint32_t>(rn) & 0x1Fu;
    std::uint32_t raw  = 0xD65F0000u | (rn_v << 5);
    return Instr{raw};
}

// ---------------------------------------------------------------------------
// Helper for Prisma's Gpr → ARM64 Reg mapping (Fase 1 start: fixed mapping).
// ---------------------------------------------------------------------------
//
// Per RFC 0001 and research notes, Fase 1 uses a fixed mapping from x86_64
// GPRs to ARM64 registers. This mirrors Box64's approach (arm64_mapping.h)
// for compilation speed, and will be replaced by constrained RA in Fase 2+.
//
// Mapping chosen:
//   x86 rax..r15  →  ARM64 x10..x25
//   (x0-x9 reserved for scratch, x26-x30 reserved for emulator state pointer,
//    flags, etc. — layout will be finalised in RFC 0003.)

[[nodiscard]] constexpr Reg host_reg_for(ir::Gpr g) noexcept {
    const auto base = static_cast<std::uint8_t>(Reg::X10);
    const auto off  = static_cast<std::uint8_t>(g);
    return static_cast<Reg>(base + off);
}

}  // namespace prisma::arm64
