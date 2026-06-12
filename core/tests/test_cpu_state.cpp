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

TEST_CASE("CpuStateFrame: fs_base and gs_base default to zero") {
    runtime::CpuStateFrame f;
    REQUIRE(f.fs_base == 0);
    REQUIRE(f.gs_base == 0);
}

TEST_CASE("CpuStateFrame: fs_base_offset and gs_base_offset are stable") {
    REQUIRE(runtime::CpuStateFrame::fs_base_offset() == 792);
    REQUIRE(runtime::CpuStateFrame::gs_base_offset() == 800);
}

TEST_CASE("CpuStateFrame: read-write fs_base and gs_base through the frame") {
    runtime::CpuStateFrame f;
    f.fs_base = 0x7F0000000000;
    f.gs_base = 0x7F0000001000;
    REQUIRE(f.fs_base == 0x7F0000000000);
    REQUIRE(f.gs_base == 0x7F0000001000);
}

TEST_CASE("CpuStateFrame: ymm_hi offsets are 400, 416, ...") {
    REQUIRE(runtime::CpuStateFrame::ymm_hi_offset_bytes(0) == 400);
    REQUIRE(runtime::CpuStateFrame::ymm_hi_offset_bytes(1) == 416);
    REQUIRE(runtime::CpuStateFrame::ymm_hi_offset_bytes(15) == 400 + 15 * 16);
}

TEST_CASE("CpuStateFrame: ymm_hi array occupies 256 bytes") {
    REQUIRE(sizeof(runtime::CpuStateFrame::ymm_hi) ==
            runtime::kYmmCount * sizeof(runtime::XmmReg));
    REQUIRE(runtime::kYmmCount == 16);
}

TEST_CASE("CpuStateFrame: total struct size exceeds 800 bytes") {
    // If this fails the layout has changed significantly.
    REQUIRE(sizeof(runtime::CpuStateFrame) >= 800);
    REQUIRE(sizeof(runtime::CpuStateFrame) <= 1024);
}

TEST_CASE("CpuStateFrame: guest_pc offset is right after GPR array") {
    // guest_pc is at offset 128 = 16 GPR × 8 bytes.
    REQUIRE(offsetof(runtime::CpuStateFrame, guest_pc) == 128);
}

TEST_CASE("CpuStateFrame: writing through offset_of produces correct values") {
    runtime::CpuStateFrame f{};
    auto* base = reinterpret_cast<std::uint8_t*>(&f);
    constexpr auto off = offsetof(runtime::CpuStateFrame, gs_base);
    const std::uint64_t val = 0xABCDEF012345678ULL;
    std::memcpy(base + off, &val, sizeof(val));
    REQUIRE(f.gs_base == val);
}

TEST_CASE("CpuStateFrame: GPR operator[] const does not crash with R15") {
    const runtime::CpuStateFrame f{};
    REQUIRE(f[ir::Gpr::R15] == 0);
    // All GPRs are zero on default construction.
    REQUIRE(f[ir::Gpr::Rax] == 0);
    REQUIRE(f[ir::Gpr::Rsp] == 0);
}

TEST_CASE("CpuStateFrame: setting and reading all GPRs") {
    runtime::CpuStateFrame f{};
    for (std::size_t i = 0; i < ir::kGprCount; ++i) {
        f[static_cast<ir::Gpr>(i)] = static_cast<std::uint64_t>(0x1000 + i);
    }
    for (std::size_t i = 0; i < ir::kGprCount; ++i) {
        REQUIRE(f[static_cast<ir::Gpr>(i)] == static_cast<std::uint64_t>(0x1000 + i));
    }
}

TEST_CASE("CpuStateFrame: is_standard_layout and trivially_copyable") {
    REQUIRE(std::is_standard_layout_v<runtime::CpuStateFrame>);
    REQUIRE(std::is_trivially_copyable_v<runtime::CpuStateFrame>);
}

TEST_CASE("CpuStateFrame: XmmReg is standard layout and trivially copyable") {
    REQUIRE(std::is_standard_layout_v<runtime::XmmReg>);
    REQUIRE(std::is_trivially_copyable_v<runtime::XmmReg>);
    REQUIRE(sizeof(runtime::XmmReg) == 16);
    REQUIRE(alignof(runtime::XmmReg) == 16);
}

TEST_CASE("CpuStateFrame: XmmReg lo/hi operations") {
    runtime::XmmReg r;
    REQUIRE(r.lo == 0);
    REQUIRE(r.hi == 0);
    r.lo = 0xAABBCCDD00112233ULL;
    r.hi = 0xFFEEDDCC44332211ULL;
    REQUIRE(r.lo == 0xAABBCCDD00112233ULL);
    REQUIRE(r.hi == 0xFFEEDDCC44332211ULL);
}

TEST_CASE("CpuStateFrame: zero-initialised frame has consistent mxcsr and TLS") {
    const runtime::CpuStateFrame f{};
    REQUIRE(f.mxcsr == 0x1F80u);
    REQUIRE(f.fs_base == 0);
    REQUIRE(f.gs_base == 0);
    REQUIRE(f.x87_status_control == 0);
}

TEST_CASE("CpuStateFrame: xmm[0..15] are all zero after value initialisation") {
    const runtime::CpuStateFrame f{};
    for (const auto& x : f.xmm) {
        REQUIRE(x.lo == 0);
        REQUIRE(x.hi == 0);
    }
}

TEST_CASE("CpuStateFrame: ymm_hi[0..15] are all zero after value initialisation") {
    const runtime::CpuStateFrame f{};
    for (const auto& y : f.ymm_hi) {
        REQUIRE(y.lo == 0);
        REQUIRE(y.hi == 0);
    }
}
