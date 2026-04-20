// core/tests/test_arm64_encoding.cpp — validate hand-rolled ARM64 encoders
// against known-correct bit patterns.
//
// Each test cites the ARM Architecture Reference Manual section and shows
// the expected encoding, so a reviewer can verify without the manual open.

#include <catch2/catch_test_macros.hpp>

#include "prisma/arm64_encoding.hpp"

using namespace prisma::arm64;
namespace ir = prisma::ir;

TEST_CASE("movz x0, #42 encodes to 0xD2800540  (ARM ARM C6.2.195)") {
    // Expected layout (64-bit variant, hw=0):
    //   sf=1 | opc=10 | 100101 | hw=00 | imm16=0x2A | Rd=0
    //   =  1   10   100101   00   000000000000101010   00000
    //   = 1101 0010 1000 0000 0000 0101 0100 0000
    //   = 0xD2 80 05 40
    constexpr Instr got = movz_x(Reg::X0, 42, 0);
    REQUIRE(got.raw == 0xD280'0540u);
}

TEST_CASE("movz x0, #0xBEEF encodes correctly") {
    // imm16 = 0xBEEF, hw=0
    //   = 1101 0010 1 00 1011111011101111 00000
    //   = 1101 0010 1001 0111 1101 1101 1110 0000
    //   = 0xD2 97 DD E0
    constexpr Instr got = movz_x(Reg::X0, 0xBEEFu, 0);
    REQUIRE(got.raw == 0xD297'DDE0u);
}

TEST_CASE("movz x1, #0xBEEF, lsl #16 encodes with hw=1") {
    // hw=1 means imm16 shifted left by 16.
    // same imm16, rd=1, hw=01
    //   = 1101 0010 101 01011111011101111 00001
    // Let's compute directly:
    //   base = 0xD2A0'0000 (sf=1, opc=10, 100101, hw=01)
    //   | (imm16 << 5) = 0xBEEF << 5 = 0x17DDE0
    //   | rd = 1
    //   = 0xD2A0'0000 | 0x17DDE0 | 0x1 = 0xD2B7'DDE1
    constexpr Instr got = movz_x(Reg::X1, 0xBEEFu, 1);
    REQUIRE(got.raw == 0xD2B7'DDE1u);
}

TEST_CASE("ret defaults to x30 and encodes to 0xD65F03C0  (ARM ARM C6.2.279)") {
    // Fixed bits + Rn=30 shifted by 5.
    //   0xD65F'0000 | (30 << 5) = 0xD65F'0000 | 0x3C0 = 0xD65F'03C0
    constexpr Instr got = ret();
    REQUIRE(got.raw == 0xD65F'03C0u);
}

TEST_CASE("ret x0 encodes to 0xD65F0000") {
    constexpr Instr got = ret(Reg::X0);
    REQUIRE(got.raw == 0xD65F'0000u);
}

TEST_CASE("Host-register mapping matches Box64-inspired fixed layout") {
    // rax → x10, rcx → x11, ..., r15 → x25. This is the mapping chosen in
    // research_notes.md decision #3 for Fase 1 (constrained RA arrives later).
    REQUIRE(host_reg_for(ir::Gpr::Rax) == Reg::X10);
    REQUIRE(host_reg_for(ir::Gpr::Rcx) == Reg::X11);
    REQUIRE(host_reg_for(ir::Gpr::R15) == Reg::X25);
}

TEST_CASE("Prisma's first end-to-end bytecode: load 42 and return") {
    // This is the ARM64 body of a function equivalent to:
    //   uint64_t f() { return 42; }
    //
    // movz x0, #42    ; 0xD2800540
    // ret             ; 0xD65F03C0
    //
    // When Fase 1 implements the emitter, the first JIT-executable program
    // will produce these exact 8 bytes. This test is the ground truth.
    constexpr Instr prog[2] = {
        movz_x(Reg::X0, 42, 0),
        ret(),
    };
    REQUIRE(prog[0].raw == 0xD280'0540u);
    REQUIRE(prog[1].raw == 0xD65F'03C0u);
}
