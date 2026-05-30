// core/tests/test_cpu_state.cpp — F1-RT-012 CpuStateFrame layout tests.

#include <catch2/catch_test_macros.hpp>
#include <cstdint>
#include <cstring>

#include "prisma/cpu_state.hpp"
#include "prisma/ir.hpp"

using namespace prisma;

TEST_CASE("CpuStateFrame: GPR offsets are 0, 8, 16, ...") {
    REQUIRE(runtime::CpuStateFrame::gpr_offset_bytes(ir::Gpr::Rax) == 0);
    REQUIRE(runtime::CpuStateFrame::gpr_offset_bytes(ir::Gpr::Rcx) == 8);
    REQUIRE(runtime::CpuStateFrame::gpr_offset_bytes(ir::Gpr::R15) == 15 * 8);
}

TEST_CASE("CpuStateFrame: GPR read/write through operator[]") {
    runtime::CpuStateFrame f;
    f[ir::Gpr::Rax] = 0xCAFEBABEu;
    f[ir::Gpr::R15] = 0xDEADBEEFu;
    REQUIRE(f[ir::Gpr::Rax] == 0xCAFEBABEu);
    REQUIRE(f[ir::Gpr::R15] == 0xDEADBEEFu);
}

TEST_CASE("CpuStateFrame: xmm[] is 16 entries × 16 bytes (256 bytes total)") {
    REQUIRE(sizeof(runtime::CpuStateFrame::xmm) ==
            runtime::kXmmCount * sizeof(runtime::XmmReg));
    REQUIRE(runtime::kXmmCount == 16);
    REQUIRE(sizeof(runtime::XmmReg) == 16);
}

TEST_CASE("CpuStateFrame: xmm_offset_bytes returns 144, 160, ...") {
    REQUIRE(runtime::CpuStateFrame::xmm_offset_bytes(0) == 144);
    REQUIRE(runtime::CpuStateFrame::xmm_offset_bytes(1) == 160);
    REQUIRE(runtime::CpuStateFrame::xmm_offset_bytes(15) == 144 + 15 * 16);
}

TEST_CASE("CpuStateFrame: write to xmm via offset and read it back") {
    runtime::CpuStateFrame f;
    auto* base = reinterpret_cast<std::uint8_t*>(&f);
    const std::int32_t off = runtime::CpuStateFrame::xmm_offset_bytes(3);
    const std::uint64_t lo = 0x0123456789ABCDEFull;
    const std::uint64_t hi = 0xFEDCBA9876543210ull;
    std::memcpy(base + off,     &lo, sizeof lo);
    std::memcpy(base + off + 8, &hi, sizeof hi);
    REQUIRE(f.xmm[3].lo == lo);
    REQUIRE(f.xmm[3].hi == hi);
}

TEST_CASE("CpuStateFrame: x87 stack is 8 deep × 16 bytes per slot") {
    REQUIRE(sizeof(runtime::CpuStateFrame::x87) ==
            runtime::kX87StackDepth * sizeof(runtime::X87Slot));
    REQUIRE(runtime::kX87StackDepth == 8);
    REQUIRE(sizeof(runtime::X87Slot) == 16);
}

TEST_CASE("CpuStateFrame: x87 offsets expose stack base and TOS byte") {
    REQUIRE(runtime::CpuStateFrame::x87_offset_bytes(0) == 656);
    REQUIRE(runtime::CpuStateFrame::x87_offset_bytes(1) == 672);
    REQUIRE(runtime::CpuStateFrame::x87_offset_bytes(7) == 656 + 7 * 16);
    REQUIRE(runtime::CpuStateFrame::kX87StatusControlOffset == 784);
    REQUIRE(runtime::CpuStateFrame::kX87TosByteOffset == 788);
}

TEST_CASE("CpuStateFrame: MXCSR defaults to 0x1F80 (mask all exceptions)") {
    runtime::CpuStateFrame f;
    REQUIRE(f.mxcsr == 0x1F80u);
}
