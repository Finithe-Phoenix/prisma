// core/tests/test_e2e_dispatcher.cpp — end-to-end x86 → ARM64
// execution tests using Translator + Dispatcher.
//
// These tests exercise the full pipeline that `prisma_run` exposes
// to users: hand-crafted x86_64 byte sequences flow through decoder
// → IR → 12-pass pipeline → ARM64 lowering → MAP_JIT execute, and
// the final guest CPU state is asserted.
//
// Apple silicon hosts only — the JIT can only run on the host arch
// we lower to. Skipped on other hosts via the `is_arm64` guard.

#include <catch2/catch_test_macros.hpp>
#include <cstdint>
#include <cstring>
#include <span>
#include <vector>

#include "prisma/cpu_state.hpp"
#include "prisma/dispatcher.hpp"
#include "prisma/host_features.hpp"
#include "prisma/translator.hpp"

using namespace prisma;

namespace {

constexpr bool is_arm64 =
#if defined(__aarch64__) || defined(__arm64__)
    true;
#else
    false;
#endif

// Thin helper that runs a byte blob through Translator+Dispatcher
// starting at guest PC 0x4000 and returns the final CpuStateFrame.
runtime::CpuStateFrame run_blob(std::vector<std::uint8_t> bytes) {
    translator::Translator tx;
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        constexpr std::uint64_t base = 0x4000;
        if (pc < base) return {};
        const std::size_t off = static_cast<std::size_t>(pc - base);
        if (off >= bytes.size()) return {};
        return std::span<const std::uint8_t>(bytes.data() + off,
                                             bytes.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto r = disp.run(0x4000, /*max_steps=*/100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    return disp.state();
}

}  // namespace

TEST_CASE("e2e: a single RET halts cleanly with all GPRs zero") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }

    auto state = run_blob({0xC3});  // ret
    REQUIRE(state[ir::Gpr::Rax] == 0);
    REQUIRE(state[ir::Gpr::Rcx] == 0);
}

TEST_CASE("e2e: mov rax, 0xDEADBEEF; ret leaves rax = 0xDEADBEEF") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }

    auto state = run_blob({
        0x48, 0xB8,                                       // mov rax, imm64
        0xEF, 0xBE, 0xAD, 0xDE, 0x00, 0x00, 0x00, 0x00,  // 0xDEADBEEF
        0xC3,                                             // ret
    });
    REQUIRE(state[ir::Gpr::Rax] == 0xDEADBEEFu);
}

TEST_CASE("e2e: mov rax,10; mov rcx,32; add rax,rcx; ret yields rax=42") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }

    auto state = run_blob({
        0x48, 0xB8, 0x0A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  // mov rax, 10
        0x48, 0xB9, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  // mov rcx, 32
        0x48, 0x01, 0xC8,                                            // add rax, rcx
        0xC3,                                                        // ret
    });
    REQUIRE(state[ir::Gpr::Rax] == 42u);
    REQUIRE(state[ir::Gpr::Rcx] == 32u);
}

TEST_CASE("e2e: subtract — mov rax,100; mov rcx,7; sub rax,rcx; ret = 93") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }

    auto state = run_blob({
        0x48, 0xB8, 0x64, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  // mov rax, 100
        0x48, 0xB9, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  // mov rcx, 7
        0x48, 0x29, 0xC8,                                            // sub rax, rcx
        0xC3,
    });
    REQUIRE(state[ir::Gpr::Rax] == 93u);
}

TEST_CASE("e2e: bitwise — mov rax,0xFF; mov rcx,0x0F; and rax,rcx; ret = 0x0F") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }

    auto state = run_blob({
        0x48, 0xB8, 0xFF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  // mov rax, 0xFF
        0x48, 0xB9, 0x0F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  // mov rcx, 0x0F
        0x48, 0x21, 0xC8,                                            // and rax, rcx
        0xC3,
    });
    REQUIRE(state[ir::Gpr::Rax] == 0x0Fu);
}

TEST_CASE("e2e: BMI2 PDEP deposits low source bits into mask positions") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }

    auto state = run_blob({
        0x48, 0xB9, 0x0B, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  // mov rcx, 0b1011
        0x48, 0xBA, 0x54, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  // mov rdx, 0b01010100
        0xC4, 0xE2, 0xF3, 0xF5, 0xC2,                                // pdep rax, rcx, rdx
        0xC3,
    });
    REQUIRE(state[ir::Gpr::Rax] == 0x14u);
}

TEST_CASE("e2e: BMI2 PEXT extracts masked source bits into low bits") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }

    auto state = run_blob({
        0x48, 0xB9, 0x54, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  // mov rcx, 0b01010100
        0x48, 0xBA, 0x54, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  // mov rdx, 0b01010100
        0xC4, 0xE2, 0xF2, 0xF5, 0xC2,                                // pext rax, rcx, rdx
        0xC3,
    });
    REQUIRE(state[ir::Gpr::Rax] == 0x7u);
}

TEST_CASE("e2e: xor reg,reg zeros the register (idiomatic x86)") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }

    auto state = run_blob({
        0x48, 0xB8, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,  // mov rax, -1
        0x48, 0x31, 0xC0,                                            // xor rax, rax
        0xC3,
    });
    REQUIRE(state[ir::Gpr::Rax] == 0u);
}

TEST_CASE("e2e: register move — mov rcx, rax preserves value") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }

    auto state = run_blob({
        0x48, 0xB8, 0x37, 0x13, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  // mov rax, 0x1337
        0x48, 0x89, 0xC1,                                            // mov rcx, rax
        0xC3,
    });
    REQUIRE(state[ir::Gpr::Rax] == 0x1337u);
    REQUIRE(state[ir::Gpr::Rcx] == 0x1337u);
}

TEST_CASE("e2e: chained ALU — (5+3)*2 = 16 via add then add (no MUL yet)") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }

    // mov rax, 5; mov rcx, 3; add rax, rcx; add rax, rax; ret  → 16
    auto state = run_blob({
        0x48, 0xB8, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x48, 0xB9, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x48, 0x01, 0xC8,                                            // add rax, rcx → 8
        0x48, 0x01, 0xC0,                                            // add rax, rax → 16
        0xC3,
    });
    REQUIRE(state[ir::Gpr::Rax] == 16u);
}

TEST_CASE("e2e: stats — single-block program reports blocks_executed = 1") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }

    translator::Translator tx;
    std::vector<std::uint8_t> bytes{
        0x48, 0xB8, 0x42, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= bytes.size()) return {};
        return std::span<const std::uint8_t>(bytes.data() + off,
                                             bytes.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(r.stats.blocks_executed == 1);
    REQUIRE(disp.state()[ir::Gpr::Rax] == 0x42u);
}

TEST_CASE("e2e: PXOR xmm0, xmm0 zeroes xmm0 (idiomatic SSE2 zero)") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }

    // Pre-load xmm0 with a known non-zero pattern via the
    // CpuStateFrame, run `pxor xmm0, xmm0; ret`, and read the
    // low lane back. Expect the lane to be zero — this is the
    // canonical "PXOR sets register to zero" idiom every SSE2
    // binary uses dozens of times.
    translator::Translator tx;
    std::vector<std::uint8_t> bytes{
        0x66, 0x0F, 0xEF, 0xC0,  // pxor xmm0, xmm0
        0xC3,                     // ret
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= bytes.size()) return {};
        return std::span<const std::uint8_t>(bytes.data() + off,
                                             bytes.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    disp.state().xmm[0].lo = 0xDEADBEEFCAFEBABEull;

    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[0].lo == 0u);
}

TEST_CASE("e2e: ADDPS xmm0, xmm1 — packed-FP add lanes-wise (4×f32)") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }

    // Load xmm0 with {1.0, 2.0, 3.0, 4.0} (low pair → lo) and xmm1 with
    // {10.0, 20.0, 30.0, 40.0}, run `addps xmm0, xmm1; ret`, expect
    // {11.0, 22.0, 33.0, 44.0}. We compare the low pair as packed
    // u32 lanes in xmm0.lo.
    translator::Translator tx;
    std::vector<std::uint8_t> bytes{
        0x0F, 0x58, 0xC1,  // addps xmm0, xmm1
        0xC3,              // ret
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= bytes.size()) return {};
        return std::span<const std::uint8_t>(bytes.data() + off,
                                             bytes.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();

    auto pack2f = [](float lo32, float hi32) -> std::uint64_t {
        std::uint32_t a, b;
        std::memcpy(&a, &lo32, 4);
        std::memcpy(&b, &hi32, 4);
        return static_cast<std::uint64_t>(a) |
               (static_cast<std::uint64_t>(b) << 32);
    };
    disp.state().xmm[0].lo = pack2f(1.0f, 2.0f);
    disp.state().xmm[0].hi = pack2f(3.0f, 4.0f);
    disp.state().xmm[1].lo = pack2f(10.0f, 20.0f);
    disp.state().xmm[1].hi = pack2f(30.0f, 40.0f);

    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[0].lo == pack2f(11.0f, 22.0f));
    REQUIRE(disp.state().xmm[0].hi == pack2f(33.0f, 44.0f));
}

TEST_CASE("e2e: ADDSS xmm0, xmm1 — scalar-FP add preserves upper xmm bits") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }

    // ADDSS only writes bits[31:0]; bits[127:32] of xmm0 must survive
    // unchanged. We seed xmm0 with {3.0, sentinel_lo_hi32, sentinel_hi}
    // and xmm1 with {4.0, ...}, run `addss xmm0, xmm1; ret`, expect the
    // low f32 = 7.0 and the upper 96 bits = the original sentinel.
    translator::Translator tx;
    std::vector<std::uint8_t> bytes{
        0xF3, 0x0F, 0x58, 0xC1,  // addss xmm0, xmm1
        0xC3,                     // ret
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= bytes.size()) return {};
        return std::span<const std::uint8_t>(bytes.data() + off,
                                             bytes.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();

    auto enc_f32_lo = [](float v) -> std::uint64_t {
        std::uint32_t bits;
        std::memcpy(&bits, &v, 4);
        return static_cast<std::uint64_t>(bits);
    };
    constexpr std::uint64_t kSentinelLoHi32 = 0xCAFEBABE'00000000ULL;
    constexpr std::uint64_t kSentinelHi     = 0xDEADBEEFFEEDFACEULL;
    disp.state().xmm[0].lo = enc_f32_lo(3.0f) | kSentinelLoHi32;
    disp.state().xmm[0].hi = kSentinelHi;
    disp.state().xmm[1].lo = enc_f32_lo(4.0f);
    disp.state().xmm[1].hi = 0;

    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[0].lo == (enc_f32_lo(7.0f) | kSentinelLoHi32));
    REQUIRE(disp.state().xmm[0].hi == kSentinelHi);
}

TEST_CASE("e2e: VADDPS xmm2, xmm0, xmm1 — AVX-128 3-operand packed FP add") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // VEX C5 [vvvv pp] 58 D1
    //   pp = 00 (no prefix → packed single)
    //   vvvv = inverted(0) = 1111
    //   R̅ = 1
    // VEX byte: 1_1111_0_00 = 0xF8
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC5, 0xF8, 0x58, 0xD1,  // vaddps xmm2, xmm0, xmm1
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack2f = [](float a, float b) -> std::uint64_t {
        std::uint32_t aa, bb;
        std::memcpy(&aa, &a, 4); std::memcpy(&bb, &b, 4);
        return static_cast<std::uint64_t>(aa) | (static_cast<std::uint64_t>(bb) << 32);
    };
    disp.state().xmm[0].lo = pack2f(1.0f, 2.0f);
    disp.state().xmm[0].hi = pack2f(3.0f, 4.0f);
    disp.state().xmm[1].lo = pack2f(10.0f, 20.0f);
    disp.state().xmm[1].hi = pack2f(30.0f, 40.0f);
    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[2].lo == pack2f(11.0f, 22.0f));
    REQUIRE(disp.state().xmm[2].hi == pack2f(33.0f, 44.0f));
}

TEST_CASE("e2e: VPXOR xmm2, xmm0, xmm1 — F2-IR-048 AVX-128 3-operand XOR") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // VEX C5 prefix: 0xC5 [R̅ vvvv L pp]
    //   pp = 01 (66 mandatory, for VPXOR)
    //   L  = 0  (128-bit)
    //   vvvv = inverted(xmm0=0) = 1111 = 15
    //   R̅  = 1 (R=0)
    // → second VEX byte = 1_1111_0_01 = 0xF9
    // Then opcode EF (PXOR) and ModR/M D1 (xmm2 ← xmm0/xmm1).
    //   ModR/M D1 = 11_010_001 → reg=2 (dst=xmm2), rm=1 (rhs=xmm1).
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC5, 0xF9, 0xEF, 0xD1,  // vpxor xmm2, xmm0, xmm1
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    disp.state().xmm[0].lo = 0xFF00FF00FF00FF00ULL;
    disp.state().xmm[0].hi = 0x00FF00FF00FF00FFULL;
    disp.state().xmm[1].lo = 0x0F0F0F0F0F0F0F0FULL;
    disp.state().xmm[1].hi = 0xF0F0F0F0F0F0F0F0ULL;
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    // xmm2 = xmm0 XOR xmm1 → byte-wise xor
    REQUIRE(disp.state().xmm[2].lo == (0xFF00FF00FF00FF00ULL ^ 0x0F0F0F0F0F0F0F0FULL));
    REQUIRE(disp.state().xmm[2].hi == (0x00FF00FF00FF00FFULL ^ 0xF0F0F0F0F0F0F0F0ULL));
}

TEST_CASE("e2e: PBLENDVB — variable byte blend selected by xmm0 MSB (SSE4.1)") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0x66, 0x0F, 0x38, 0x10, 0xCA,  // pblendvb xmm1, xmm2 (mask = xmm0)
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    // xmm1 = dst (preserved where mask MSB == 0)
    // xmm2 = src (selected where mask MSB == 1)
    // xmm0 = mask (per byte: 0xFF picks src, 0x00 picks dst)
    auto pack = [](std::uint8_t* b) {
        std::uint64_t v = 0;
        for (int i = 0; i < 8; ++i) v |= static_cast<std::uint64_t>(b[i]) << (i*8);
        return v;
    };
    std::uint8_t dst_lo[8] = {0xD0,0xD1,0xD2,0xD3,0xD4,0xD5,0xD6,0xD7};
    std::uint8_t src_lo[8] = {0x10,0x11,0x12,0x13,0x14,0x15,0x16,0x17};
    std::uint8_t msk_lo[8] = {0xFF,0x00,0xFF,0x00,0xFF,0x00,0xFF,0x00};
    disp.state().xmm[1].lo = pack(dst_lo);
    disp.state().xmm[1].hi = pack(dst_lo);
    disp.state().xmm[2].lo = pack(src_lo);
    disp.state().xmm[2].hi = pack(src_lo);
    disp.state().xmm[0].lo = pack(msk_lo);
    disp.state().xmm[0].hi = pack(msk_lo);
    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    std::uint8_t exp[8] = {0x10,0xD1,0x12,0xD3,0x14,0xD5,0x16,0xD7};
    REQUIRE(disp.state().xmm[1].lo == pack(exp));
    REQUIRE(disp.state().xmm[1].hi == pack(exp));
}

TEST_CASE("e2e: LZCNT + TZCNT — leading/trailing zero counts (BMI1)") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xF3, 0x48, 0x0F, 0xBD, 0xC1,  // lzcnt rax, rcx
        0xF3, 0x48, 0x0F, 0xBC, 0xDA,  // tzcnt rbx, rdx
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    // 0x0000_0000_0000_0100 → leading zeros = 55, trailing zeros = 8.
    disp.state()[ir::Gpr::Rcx] = 0x0000000000000100ULL;
    disp.state()[ir::Gpr::Rdx] = 0x0000000000000100ULL;
    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state()[ir::Gpr::Rax] == 55u);
    REQUIRE(disp.state()[ir::Gpr::Rbx] == 8u);
}

TEST_CASE("e2e: LZCNT sets carry flag for a following JC") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    auto state = run_blob({
        0x48, 0xB8, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00,  // mov rax, 0
        0xF3, 0x48, 0x0F, 0xBD, 0xC8,        // lzcnt rcx, rax
        0x72, 0x0B,                          // jc +11
        0x48, 0xBA, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00,  // mov rdx, 0
        0xC3,                                // ret
        0x48, 0xBA, 0x01, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00,  // mov rdx, 1
        0xC3,                                // ret
    });
    REQUIRE(state[ir::Gpr::Rcx] == 64u);
    REQUIRE(state[ir::Gpr::Rdx] == 1u);
}

TEST_CASE("e2e: XADD sets carry flag for a following JC") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }

    auto state = run_blob({
        0x48, 0xB8, 0xFF, 0xFF, 0xFF, 0xFF,
                    0xFF, 0xFF, 0xFF, 0xFF,  // mov rax, -1
        0x48, 0xB9, 0x01, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00,  // mov rcx, 1
        0x48, 0x0F, 0xC1, 0xC8,              // xadd rax, rcx
        0x72, 0x0B,                          // jc +11
        0x48, 0xBA, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00,  // mov rdx, 0
        0xC3,                                // ret
        0x48, 0xBA, 0x01, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00,  // mov rdx, 1
        0xC3,                                // ret
    });
    REQUIRE(state[ir::Gpr::Rax] == 0u);
    REQUIRE(state[ir::Gpr::Rcx] == 0xFFFF'FFFF'FFFF'FFFFULL);
    REQUIRE(state[ir::Gpr::Rdx] == 1u);
}

TEST_CASE("e2e: POPCNT 0xCAFEBABE counts the bits") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xF3, 0x48, 0x0F, 0xB8, 0xC1,  // popcnt rax, rcx
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    // popcount(0xCAFEBABEDEADBEEF) = 42.
    disp.state()[ir::Gpr::Rcx] = 0xCAFEBABEDEADBEEFULL;
    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    int expected = __builtin_popcountll(0xCAFEBABEDEADBEEFULL);
    REQUIRE(disp.state()[ir::Gpr::Rax] == static_cast<std::uint64_t>(expected));
}

TEST_CASE("e2e: ROUNDSD truncates 7.9 to 7.0 (mode=3)") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0x66, 0x0F, 0x3A, 0x0B, 0xC0, 0x03,  // roundsd xmm0, xmm0, 3 (truncate)
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    double v = 7.9;
    std::uint64_t bits;
    std::memcpy(&bits, &v, 8);
    disp.state().xmm[0].lo = bits;
    disp.state().xmm[0].hi = 0xDEADBEEFCAFEBABEULL;  // upper preserved
    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    double exp = 7.0;
    std::uint64_t exp_bits;
    std::memcpy(&exp_bits, &exp, 8);
    REQUIRE(disp.state().xmm[0].lo == exp_bits);
    REQUIRE(disp.state().xmm[0].hi == 0xDEADBEEFCAFEBABEULL);
}

TEST_CASE("e2e: PMOVZXBW — F2-IR-041 zero-extend 8 bytes to 8 halfwords") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0x66, 0x0F, 0x38, 0x30, 0xC1,  // pmovzxbw xmm0, xmm1
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    // xmm1 low qword bytes 0..7 = {0xFF, 0x80, 0x01, 0x7F, 0xAB, 0xCD, 0x00, 0xEE}
    disp.state().xmm[1].lo = 0xEE'00'CD'AB'7F'01'80'FFULL;
    disp.state().xmm[1].hi = 0;
    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    // Each byte zero-extended to a halfword:
    //   {0x00FF, 0x0080, 0x0001, 0x007F, 0x00AB, 0x00CD, 0x0000, 0x00EE}
    auto pack4h = [](std::uint16_t a, std::uint16_t b,
                     std::uint16_t c, std::uint16_t d) -> std::uint64_t {
        return  static_cast<std::uint64_t>(a)
              | (static_cast<std::uint64_t>(b) << 16)
              | (static_cast<std::uint64_t>(c) << 32)
              | (static_cast<std::uint64_t>(d) << 48);
    };
    REQUIRE(disp.state().xmm[0].lo == pack4h(0x00FF, 0x0080, 0x0001, 0x007F));
    REQUIRE(disp.state().xmm[0].hi == pack4h(0x00AB, 0x00CD, 0x0000, 0x00EE));
}

TEST_CASE("e2e: PEXTRQ rax, xmm0, 1 — extract upper qword (SSE4.1)") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0x66, 0x48, 0x0F, 0x3A, 0x16, 0xC0, 0x01,  // pextrq rax, xmm0, 1
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    disp.state().xmm[0].lo = 0x1111111111111111ULL;
    disp.state().xmm[0].hi = 0xDEADBEEFCAFEBABEULL;
    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state()[ir::Gpr::Rax] == 0xDEADBEEFCAFEBABEULL);
}

TEST_CASE("e2e: PALIGNR concat-shift by 4 bytes (SSSE3)") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0x66, 0x0F, 0x3A, 0x0F, 0xC1, 0x04,  // palignr xmm0, xmm1, 4
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    // xmm1 holds bytes 0..15 = 0x10..0x1F.
    // xmm0 holds bytes 16..31 = 0x20..0x2F.
    // PALIGNR with count=4: result = (xmm0 || xmm1) >> (4 bytes), low 16
    //   = bytes [4, 5, ..., 15, 16, 17, 18, 19] = 0x14..0x1F, 0x20..0x23.
    disp.state().xmm[1].lo = 0x1716151413121110ULL;
    disp.state().xmm[1].hi = 0x1F1E1D1C1B1A1918ULL;
    disp.state().xmm[0].lo = 0x2726252423222120ULL;
    disp.state().xmm[0].hi = 0x2F2E2D2C2B2A2928ULL;
    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    // Expected lo = bytes 4..11 of (xmm0||xmm1) = {0x14,0x15,...,0x1B}
    // = 0x1B1A191817161514.
    // Expected hi = bytes 12..19 = {0x1C,0x1D,0x1E,0x1F,0x20,0x21,0x22,0x23}
    // = 0x232221201F1E1D1C.
    REQUIRE(disp.state().xmm[0].lo == 0x1B1A191817161514ULL);
    REQUIRE(disp.state().xmm[0].hi == 0x232221201F1E1D1CULL);
}

TEST_CASE("e2e: PSHUFB byte-permute with MSB-set zero gating (SSSE3)") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0x66, 0x0F, 0x38, 0x00, 0xC1,  // pshufb xmm0, xmm1
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack8 = [](std::uint8_t* b) {
        std::uint64_t v = 0;
        for (int i = 0; i < 8; ++i)
            v |= static_cast<std::uint64_t>(b[i]) << (i*8);
        return v;
    };
    // xmm0 source bytes: index → ascii letter A..P (0x41..0x50)
    std::uint8_t src_lo[8] = {0x41,0x42,0x43,0x44,0x45,0x46,0x47,0x48};
    std::uint8_t src_hi[8] = {0x49,0x4A,0x4B,0x4C,0x4D,0x4E,0x4F,0x50};
    // mask: select bytes 15, 0, 7, 0x80 (MSB → zero), 1, 14, 0x80, 0,
    //                    0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80
    std::uint8_t mask_lo[8] = {15, 0, 7, 0x80, 1, 14, 0x80, 0};
    std::uint8_t mask_hi[8] = {0x80,0x80,0x80,0x80,0x80,0x80,0x80,0x80};
    disp.state().xmm[0].lo = pack8(src_lo);
    disp.state().xmm[0].hi = pack8(src_hi);
    disp.state().xmm[1].lo = pack8(mask_lo);
    disp.state().xmm[1].hi = pack8(mask_hi);
    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    // Expected lane[i] = mask[i] MSB ? 0 : src[mask[i] & 0xF]
    std::uint8_t exp_lo[8] = {src_hi[7], src_lo[0], src_lo[7], 0,
                               src_lo[1], src_hi[6], 0, src_lo[0]};
    std::uint8_t exp_hi[8] = {0,0,0,0, 0,0,0,0};
    REQUIRE(disp.state().xmm[0].lo == pack8(exp_lo));
    REQUIRE(disp.state().xmm[0].hi == pack8(exp_hi));
}

TEST_CASE("e2e: CMPLTPS — F2-IR-034 packed FP less-than mask") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0x0F, 0xC2, 0xC1, 0x01,  // cmpltps xmm0, xmm1 (pred=1)
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack2f = [](float a, float b) -> std::uint64_t {
        std::uint32_t aa, bb;
        std::memcpy(&aa, &a, 4); std::memcpy(&bb, &b, 4);
        return static_cast<std::uint64_t>(aa) | (static_cast<std::uint64_t>(bb) << 32);
    };
    // xmm0 = {1,2,3,4}, xmm1 = {2,2,2,2}. ltps gives {1<2, 2<2, 3<2, 4<2}
    //   = {0xFFFFFFFF, 0, 0, 0}
    disp.state().xmm[0].lo = pack2f(1.0f, 2.0f);
    disp.state().xmm[0].hi = pack2f(3.0f, 4.0f);
    disp.state().xmm[1].lo = pack2f(2.0f, 2.0f);
    disp.state().xmm[1].hi = pack2f(2.0f, 2.0f);
    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    auto pack4 = [](std::uint32_t a, std::uint32_t b) -> std::uint64_t {
        return static_cast<std::uint64_t>(a) | (static_cast<std::uint64_t>(b) << 32);
    };
    REQUIRE(disp.state().xmm[0].lo == pack4(0xFFFFFFFFu, 0u));
    REQUIRE(disp.state().xmm[0].hi == pack4(0u, 0u));
}

TEST_CASE("e2e: MOVDDUP — F2-IR-033 broadcast low qword to both D2 lanes") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xF2, 0x0F, 0x12, 0xC1,  // movddup xmm0, xmm1
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    disp.state().xmm[1].lo = 0xCAFEBABEDEADBEEFULL;
    disp.state().xmm[1].hi = 0xFFFFFFFFFFFFFFFFULL;  // ignored
    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[0].lo == 0xCAFEBABEDEADBEEFULL);
    REQUIRE(disp.state().xmm[0].hi == 0xCAFEBABEDEADBEEFULL);
}

TEST_CASE("e2e: HADDPS — F2-IR-032 horizontal pairwise add of 4 floats") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xF2, 0x0F, 0x7C, 0xC1,  // haddps xmm0, xmm1
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack2f = [](float a, float b) -> std::uint64_t {
        std::uint32_t aa, bb;
        std::memcpy(&aa, &a, 4); std::memcpy(&bb, &b, 4);
        return static_cast<std::uint64_t>(aa) | (static_cast<std::uint64_t>(bb) << 32);
    };
    // xmm0 = {1.0, 2.0, 3.0, 4.0}, xmm1 = {10, 20, 30, 40}.
    // HADDPS: result = {1+2, 3+4, 10+20, 30+40} = {3, 7, 30, 70}.
    disp.state().xmm[0].lo = pack2f(1.0f, 2.0f);
    disp.state().xmm[0].hi = pack2f(3.0f, 4.0f);
    disp.state().xmm[1].lo = pack2f(10.0f, 20.0f);
    disp.state().xmm[1].hi = pack2f(30.0f, 40.0f);
    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[0].lo == pack2f(3.0f, 7.0f));
    REQUIRE(disp.state().xmm[0].hi == pack2f(30.0f, 70.0f));
}

TEST_CASE("e2e: PSADBW — F2-IR-031 sum of byte differences (used by memcmp)") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0x66, 0x0F, 0xF6, 0xC1,  // psadbw xmm0, xmm1
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    // xmm0 bytes 0..7  = {10, 20, 30, 40, 50, 60, 70, 80}
    // xmm1 bytes 0..7  = {15, 15, 35, 35, 55, 55, 75, 75}
    // |diffs|          = { 5,  5,  5,  5,  5,  5,  5,  5} → sum = 40 → 0x28
    // upper halves match identically → sum = 0.
    disp.state().xmm[0].lo = 0x5040302010ULL | (0x80706050ULL << 32);  // bytes 10,20,30,40,50,60,70,80
    disp.state().xmm[1].lo = 0x35350F0F0FULL ^ 0;  // need careful packing
    // Build by hand:
    auto pack8 = [](std::uint8_t* b) {
        std::uint64_t v = 0;
        for (int i = 0; i < 8; ++i)
            v |= static_cast<std::uint64_t>(b[i]) << (i*8);
        return v;
    };
    std::uint8_t xmm0_lo[8] = {10,20,30,40,50,60,70,80};
    std::uint8_t xmm1_lo[8] = {15,15,35,35,55,55,75,75};
    std::uint8_t same_hi[8] = {1,2,3,4,5,6,7,8};
    disp.state().xmm[0].lo = pack8(xmm0_lo);
    disp.state().xmm[0].hi = pack8(same_hi);
    disp.state().xmm[1].lo = pack8(xmm1_lo);
    disp.state().xmm[1].hi = pack8(same_hi);
    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[0].lo == 40u);
    REQUIRE(disp.state().xmm[0].hi == 0u);
}

TEST_CASE("e2e: PMULUDQ — F2-IR-030 unsigned 32x32→64 lane mul") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0x66, 0x0F, 0xF4, 0xC1,  // pmuludq xmm0, xmm1
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack4 = [](std::uint32_t a, std::uint32_t b) -> std::uint64_t {
        return static_cast<std::uint64_t>(a) | (static_cast<std::uint64_t>(b) << 32);
    };
    // xmm0 lanes (S4): {0x10000, 0xDEAD, 0x100, 0xBEEF}.
    // xmm1 lanes (S4): {0x100,   0xDEAD, 0x200, 0xBEEF}.
    // Expected: result.d2[0] = 0x10000 * 0x100 = 0x01000000.
    //           result.d2[1] = 0x100   * 0x200 = 0x00020000.
    disp.state().xmm[0].lo = pack4(0x10000u, 0xDEADu);
    disp.state().xmm[0].hi = pack4(0x100u,   0xBEEFu);
    disp.state().xmm[1].lo = pack4(0x100u,   0xDEADu);
    disp.state().xmm[1].hi = pack4(0x200u,   0xBEEFu);
    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[0].lo == 0x01000000ULL);
    REQUIRE(disp.state().xmm[0].hi == 0x00020000ULL);
}

TEST_CASE("e2e: MOVMSKPS extracts 4 sign bits from S4 lanes") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0x0F, 0x50, 0xC0,  // movmskps eax, xmm0
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack4 = [](std::uint32_t a, std::uint32_t b) -> std::uint64_t {
        return static_cast<std::uint64_t>(a) | (static_cast<std::uint64_t>(b) << 32);
    };
    // Lanes (S4): 0x80000000 (sign=1), 0x00000000 (0), 0xFFFFFFFF (1), 0x7FFFFFFF (0).
    // Expected mask = bits 0,2 set = 0x5.
    disp.state().xmm[0].lo = pack4(0x80000000u, 0x00000000u);
    disp.state().xmm[0].hi = pack4(0xFFFFFFFFu, 0x7FFFFFFFu);
    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state()[ir::Gpr::Rax] == 0x5u);
}

TEST_CASE("e2e: PSHUFLW reverses low 4 words, high half passes through") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xF2, 0x0F, 0x70, 0xC1, 0x1B,  // pshuflw xmm0, xmm1, 0x1B = lanes [3,2,1,0]
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack4 = [](std::uint16_t a, std::uint16_t b,
                    std::uint16_t c, std::uint16_t d) -> std::uint64_t {
        return  static_cast<std::uint64_t>(a)
              | (static_cast<std::uint64_t>(b) << 16)
              | (static_cast<std::uint64_t>(c) << 32)
              | (static_cast<std::uint64_t>(d) << 48);
    };
    disp.state().xmm[1].lo = pack4(0x1111, 0x2222, 0x3333, 0x4444);
    disp.state().xmm[1].hi = pack4(0x5555, 0x6666, 0x7777, 0x8888);
    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[0].lo == pack4(0x4444, 0x3333, 0x2222, 0x1111));
    REQUIRE(disp.state().xmm[0].hi == pack4(0x5555, 0x6666, 0x7777, 0x8888));
}

TEST_CASE("e2e: PMOVMSKB eax, xmm0 — F2-IR-027 byte-MSB extraction") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0x66, 0x0F, 0xD7, 0xC0,  // pmovmskb eax, xmm0
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    // Bytes (little-endian byte order in lo/hi):
    //   lo bytes 0..7: 0x80 (MSB=1), 0x01 (MSB=0), 0xFF (1), 0x7F (0), 0x80 (1), 0x00 (0), 0xC0 (1), 0x40 (0)
    //   hi bytes 8..15: 0x80, 0x80, 0x00, 0x00, 0xFF, 0xFF, 0x01, 0x80
    // Expected mask bits (LSB first per byte index):
    //   bits 0..7  = 1 0 1 0 1 0 1 0  → 0x55
    //   bits 8..15 = 1 1 0 0 1 1 0 1  → 0xB3
    // Mask = 0xB355.
    disp.state().xmm[0].lo = 0x40C0008080FF0180ULL;  // bytes 0..7 reversed for little-endian layout
    disp.state().xmm[0].hi = 0x80'01'FF'FF'00'00'80'80ULL;
    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    // Recompute the expected mask from the bytes we set.
    auto compute_mask = [&]() {
        std::uint16_t mask = 0;
        std::uint8_t bytes[16];
        std::memcpy(&bytes[0], &disp.state().xmm[0].lo, 8);
        std::memcpy(&bytes[8], &disp.state().xmm[0].hi, 8);
        for (int i = 0; i < 16; ++i) {
            if (bytes[i] & 0x80u) mask |= static_cast<std::uint16_t>(1u << i);
        }
        return mask;
    };
    const std::uint64_t expected = compute_mask();
    REQUIRE(disp.state()[ir::Gpr::Rax] == expected);
}

TEST_CASE("e2e: PMULHW xmm0, xmm1 — F2-IR-025 signed high-half multiply") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0x66, 0x0F, 0xE5, 0xC1,  // pmulhw xmm0, xmm1
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack4h = [](std::int16_t a, std::int16_t b,
                     std::int16_t c, std::int16_t d) -> std::uint64_t {
        return  static_cast<std::uint64_t>(static_cast<std::uint16_t>(a))
              | (static_cast<std::uint64_t>(static_cast<std::uint16_t>(b)) << 16)
              | (static_cast<std::uint64_t>(static_cast<std::uint16_t>(c)) << 32)
              | (static_cast<std::uint64_t>(static_cast<std::uint16_t>(d)) << 48);
    };
    // 0x4000 * 0x0002 = 0x0000_8000 → high16 = 0x0000.
    // 0x4000 * 0x4000 = 0x1000_0000 → high16 = 0x1000.
    // -1   * -1       = 0x0000_0001 → high16 = 0x0000.
    // 0x7FFF * 0x7FFF = 0x3FFF_0001 → high16 = 0x3FFF.
    disp.state().xmm[0].lo = pack4h(0x4000, 0x4000, -1, 0x7FFF);
    disp.state().xmm[0].hi = pack4h(0, 0, 0, 0);
    disp.state().xmm[1].lo = pack4h(0x0002, 0x4000, -1, 0x7FFF);
    disp.state().xmm[1].hi = pack4h(0, 0, 0, 0);

    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[0].lo == pack4h(0x0000, 0x1000, 0x0000, 0x3FFF));
}

TEST_CASE("e2e: PADDUSB clamps at 0xFF — F2-IR-023 unsigned sat add") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0x66, 0x0F, 0xDC, 0xC1,  // paddusb xmm0, xmm1
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    // Each byte: 0xF0 + 0x20 = 0x110 → clamp to 0xFF.
    disp.state().xmm[0].lo = 0xF0F0F0F0F0F0F0F0ULL;
    disp.state().xmm[0].hi = 0xF0F0F0F0F0F0F0F0ULL;
    disp.state().xmm[1].lo = 0x2020202020202020ULL;
    disp.state().xmm[1].hi = 0x2020202020202020ULL;
    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[0].lo == 0xFFFFFFFFFFFFFFFFULL);
    REQUIRE(disp.state().xmm[0].hi == 0xFFFFFFFFFFFFFFFFULL);
}

TEST_CASE("e2e: PEXTRW eax, xmm0, 3 — F2-IR-022 word extract from lane 3") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0x66, 0x0F, 0xC5, 0xC0, 0x03,  // pextrw eax, xmm0, 3
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    // xmm0 lanes (H8): {0x1111, 0x2222, 0x3333, 0x4444, 0x5555, 0x6666, 0x7777, 0x8888}.
    disp.state().xmm[0].lo = 0x4444'3333'2222'1111ULL;
    disp.state().xmm[0].hi = 0x8888'7777'6666'5555ULL;
    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state()[ir::Gpr::Rax] == 0x4444u);
}

TEST_CASE("e2e: MOVSS reg-reg preserves upper xmm bits") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xF3, 0x0F, 0x10, 0xC1,  // movss xmm0, xmm1
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    constexpr std::uint64_t kSentinelLoHi32 = 0xCAFEBABE'00000000ULL;
    constexpr std::uint64_t kSentinelHi     = 0xDEADBEEFFEEDFACEULL;
    disp.state().xmm[0].lo = 0x1111'2222ULL | kSentinelLoHi32;
    disp.state().xmm[0].hi = kSentinelHi;
    disp.state().xmm[1].lo = 0xAAAA'BBBBULL;
    disp.state().xmm[1].hi = 0xFFFFFFFFFFFFFFFFULL;
    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    // Low 32 bits = xmm1 low; upper 96 bits = xmm0 sentinel.
    REQUIRE(disp.state().xmm[0].lo == (0xAAAABBBBULL | kSentinelLoHi32));
    REQUIRE(disp.state().xmm[0].hi == kSentinelHi);
}

TEST_CASE("e2e: SHUFPS xmm0, xmm1, 0x4E — F2-IR-020 two-source FP shuffle") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // shufps xmm0, xmm1, 0x4E: control = 01_00_11_10
    //   result.s[0] = xmm0.s[(0x4E >> 0) & 3] = xmm0.s[2]
    //   result.s[1] = xmm0.s[(0x4E >> 2) & 3] = xmm0.s[3]
    //   result.s[2] = xmm1.s[(0x4E >> 4) & 3] = xmm1.s[0]
    //   result.s[3] = xmm1.s[(0x4E >> 6) & 3] = xmm1.s[1]
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0x0F, 0xC6, 0xC1, 0x4E,  // shufps xmm0, xmm1, 0x4E
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack4 = [](std::uint32_t a, std::uint32_t b) -> std::uint64_t {
        return static_cast<std::uint64_t>(a) | (static_cast<std::uint64_t>(b) << 32);
    };
    disp.state().xmm[0].lo = pack4(0xA0u, 0xA1u);
    disp.state().xmm[0].hi = pack4(0xA2u, 0xA3u);
    disp.state().xmm[1].lo = pack4(0xB0u, 0xB1u);
    disp.state().xmm[1].hi = pack4(0xB2u, 0xB3u);
    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    // Expected: {a2, a3, b0, b1}
    REQUIRE(disp.state().xmm[0].lo == pack4(0xA2u, 0xA3u));
    REQUIRE(disp.state().xmm[0].hi == pack4(0xB0u, 0xB1u));
}

TEST_CASE("e2e: SQRTPS — packed sqrt of 4 floats") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0x0F, 0x51, 0xC1,  // sqrtps xmm0, xmm1
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack2f = [](float a, float b) -> std::uint64_t {
        std::uint32_t aa, bb;
        std::memcpy(&aa, &a, 4); std::memcpy(&bb, &b, 4);
        return static_cast<std::uint64_t>(aa) | (static_cast<std::uint64_t>(bb) << 32);
    };
    disp.state().xmm[1].lo = pack2f(4.0f, 9.0f);
    disp.state().xmm[1].hi = pack2f(16.0f, 25.0f);
    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[0].lo == pack2f(2.0f, 3.0f));
    REQUIRE(disp.state().xmm[0].hi == pack2f(4.0f, 5.0f));
}

TEST_CASE("e2e: SQRTSD round-trip — F2-IR-019 sqrt(16.0) == 4.0") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // mov rax, 16; cvtsi2sd xmm0, rax; sqrtsd xmm0, xmm0; cvttsd2si rcx, xmm0; ret.
    // Round-trip: rcx = 4.
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xF2, 0x48, 0x0F, 0x2A, 0xC0,    // cvtsi2sd xmm0, rax
        0xF2, 0x0F, 0x51, 0xC0,          // sqrtsd xmm0, xmm0
        0xF2, 0x48, 0x0F, 0x2C, 0xC8,    // cvttsd2si rcx, xmm0
        0xC3,                             // ret
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    disp.state()[ir::Gpr::Rax] = 16u;
    auto r = disp.run(0x4000, 100);
    INFO("dispatch message: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state()[ir::Gpr::Rcx] == 4u);
}

TEST_CASE("e2e: MAXSD selects the larger double") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xF2, 0x0F, 0x5F, 0xC1,    // maxsd xmm0, xmm1
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto bits_of = [](double v) -> std::uint64_t {
        std::uint64_t b; std::memcpy(&b, &v, 8); return b;
    };
    disp.state().xmm[0].lo = bits_of(3.0);
    disp.state().xmm[0].hi = 0;
    disp.state().xmm[1].lo = bits_of(7.5);
    disp.state().xmm[1].hi = 0;
    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[0].lo == bits_of(7.5));
}

TEST_CASE("e2e: XORPS xmm0, xmm0 — F2-IR-018 idiomatic FP zero") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0x0F, 0x57, 0xC0,  // xorps xmm0, xmm0
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    disp.state().xmm[0].lo = 0xDEADBEEFCAFEBABEull;
    disp.state().xmm[0].hi = 0xFEEDFACEDEADBEEFull;
    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[0].lo == 0u);
    REQUIRE(disp.state().xmm[0].hi == 0u);
}

TEST_CASE("e2e: CVTSS2SD chain — F2-IR-017 single→double round trip via int") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // cvtsi2ss xmm0, eax; cvtss2sd xmm0, xmm0; cvttsd2si rcx, xmm0; ret.
    // RAX=7 → xmm0=f32(7.0) → xmm0=f64(7.0) → rcx=7.
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xF3, 0x0F, 0x2A, 0xC0,  // cvtsi2ss xmm0, eax
        0xF3, 0x0F, 0x5A, 0xC0,  // cvtss2sd xmm0, xmm0
        0xF2, 0x48, 0x0F, 0x2C, 0xC8,  // cvttsd2si rcx, xmm0
        0xC3,                     // ret
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    disp.state()[ir::Gpr::Rax] = 7u;
    auto r = disp.run(0x4000, 100);
    INFO("dispatch message: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state()[ir::Gpr::Rcx] == 7u);
}

TEST_CASE("e2e: CVTSI2SD + CVTTSD2SI — F2-IR-016 round-trip int→f64→int") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // mov rax, 42; cvtsi2sd xmm0, rax; cvttsd2si rcx, xmm0; ret.
    // Round-trip: rcx ends up = 42.
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xF2, 0x48, 0x0F, 0x2A, 0xC0,  // cvtsi2sd xmm0, rax
        0xF2, 0x48, 0x0F, 0x2C, 0xC8,  // cvttsd2si rcx, xmm0
        0xC3,                           // ret
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    disp.state()[ir::Gpr::Rax] = 42u;
    auto r = disp.run(0x4000, 100);
    INFO("dispatch message: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state()[ir::Gpr::Rcx] == 42u);
}

TEST_CASE("e2e: PSRLDQ xmm0, 8 — F2-IR-014 byte-shift right by 8") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // psrldq xmm0, 8: bytes shift right by 8 → upper half moves into
    // lower half, upper half becomes zero.
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0x66, 0x0F, 0x73, 0xD8, 0x08,  // psrldq xmm0, 8
        0xC3,                            // ret
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    disp.state().xmm[0].lo = 0x1122334455667788ull;
    disp.state().xmm[0].hi = 0xAABBCCDDEEFF0011ull;

    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[0].lo == 0xAABBCCDDEEFF0011ull);
    REQUIRE(disp.state().xmm[0].hi == 0x0ull);
}

TEST_CASE("e2e: PMULLW xmm0, xmm1 — F2-IR-013 lane-wise i16 multiply") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // pmullw xmm0, xmm1: lane-wise low-16 of (xmm0[i] * xmm1[i]).
    // xmm0 lanes (i16): {2,3,4,5,6,7,8,9}; xmm1: {10,10,...,10}.
    // Expect xmm0: {20,30,40,50,60,70,80,90}.
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0x66, 0x0F, 0xD5, 0xC1,  // pmullw xmm0, xmm1
        0xC3,                     // ret
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack8 = [](std::uint16_t a, std::uint16_t b,
                    std::uint16_t c, std::uint16_t d) -> std::uint64_t {
        return  static_cast<std::uint64_t>(a)
              | (static_cast<std::uint64_t>(b) << 16)
              | (static_cast<std::uint64_t>(c) << 32)
              | (static_cast<std::uint64_t>(d) << 48);
    };
    disp.state().xmm[0].lo = pack8(2, 3, 4, 5);
    disp.state().xmm[0].hi = pack8(6, 7, 8, 9);
    disp.state().xmm[1].lo = pack8(10, 10, 10, 10);
    disp.state().xmm[1].hi = pack8(10, 10, 10, 10);

    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[0].lo == pack8(20, 30, 40, 50));
    REQUIRE(disp.state().xmm[0].hi == pack8(60, 70, 80, 90));
}

TEST_CASE("e2e: PUNPCKLDQ xmm0, xmm1 — F2-IR-011 32-bit interleave low") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // xmm0 = {a0,a1,a2,a3} (lanes 0..3), xmm1 = {b0,b1,b2,b3}.
    // PUNPCKLDQ takes the bottom-half lanes (a0,a1,b0,b1) interleaved
    // pairwise → xmm0 = {a0, b0, a1, b1}.
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0x66, 0x0F, 0x62, 0xC1,  // punpckldq xmm0, xmm1
        0xC3,                     // ret
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack4 = [](std::uint32_t a, std::uint32_t b) -> std::uint64_t {
        return static_cast<std::uint64_t>(a) |
               (static_cast<std::uint64_t>(b) << 32);
    };
    disp.state().xmm[0].lo = pack4(0xA0u, 0xA1u);
    disp.state().xmm[0].hi = pack4(0xA2u, 0xA3u);
    disp.state().xmm[1].lo = pack4(0xB0u, 0xB1u);
    disp.state().xmm[1].hi = pack4(0xB2u, 0xB3u);

    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[0].lo == pack4(0xA0u, 0xB0u));
    REQUIRE(disp.state().xmm[0].hi == pack4(0xA1u, 0xB1u));
}

TEST_CASE("e2e: PSLLD xmm0, 4 — F2-IR-012 per-lane left shift") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0x66, 0x0F, 0x72, 0xF0, 0x04,  // pslld xmm0, 4
        0xC3,                            // ret
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack4 = [](std::uint32_t a, std::uint32_t b) -> std::uint64_t {
        return static_cast<std::uint64_t>(a) |
               (static_cast<std::uint64_t>(b) << 32);
    };
    disp.state().xmm[0].lo = pack4(0x01u, 0x02u);
    disp.state().xmm[0].hi = pack4(0x03u, 0x04u);

    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[0].lo == pack4(0x10u, 0x20u));
    REQUIRE(disp.state().xmm[0].hi == pack4(0x30u, 0x40u));
}

TEST_CASE("e2e: PSHUFD xmm0, xmm1, 0x1B — F2-IR-010 lane reversal") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // pshufd xmm0, xmm1, 0x1B (= binary 00_01_10_11 = lanes [3,2,1,0]).
    // xmm1 = {1,2,3,4}, expect xmm0 = {4,3,2,1}.
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0x66, 0x0F, 0x70, 0xC1, 0x1B,  // pshufd xmm0, xmm1, 0x1B
        0xC3,                            // ret
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack4 = [](std::uint32_t a, std::uint32_t b) -> std::uint64_t {
        return static_cast<std::uint64_t>(a) |
               (static_cast<std::uint64_t>(b) << 32);
    };
    disp.state().xmm[1].lo = pack4(1u, 2u);
    disp.state().xmm[1].hi = pack4(3u, 4u);

    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[0].lo == pack4(4u, 3u));
    REQUIRE(disp.state().xmm[0].hi == pack4(2u, 1u));
}

TEST_CASE("e2e: PCMPEQD xmm0, xmm1 — F2-IR-009 lane-wise equality") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // pcmpeqd xmm0, xmm1; ret. xmm0 = {1,2,3,4}, xmm1 = {1,99,3,99}.
    // Expect xmm0 lanes 0 and 2 = 0xFFFFFFFF, lanes 1 and 3 = 0.
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0x66, 0x0F, 0x76, 0xC1,  // pcmpeqd xmm0, xmm1
        0xC3,                     // ret
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack4 = [](std::uint32_t a, std::uint32_t b) -> std::uint64_t {
        return static_cast<std::uint64_t>(a) |
               (static_cast<std::uint64_t>(b) << 32);
    };
    disp.state().xmm[0].lo = pack4(1u, 2u);
    disp.state().xmm[0].hi = pack4(3u, 4u);
    disp.state().xmm[1].lo = pack4(1u, 99u);
    disp.state().xmm[1].hi = pack4(3u, 99u);

    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[0].lo == pack4(0xFFFFFFFFu, 0u));
    REQUIRE(disp.state().xmm[0].hi == pack4(0xFFFFFFFFu, 0u));
}

TEST_CASE("e2e: MOVQ xmm0, rax + MOVQ rcx, xmm0 — F2-IR-008 GPR↔XMM transfers") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // mov rax, 0xCAFEBABEDEADBEEF; movq xmm0, rax; movq rcx, xmm0; ret.
    // Round-trip through xmm0 — RCX must end up = the seeded constant.
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0x48, 0xB8, 0xEF, 0xBE, 0xAD, 0xDE, 0xBE, 0xBA, 0xFE, 0xCA,  // mov rax, imm64
        0x66, 0x48, 0x0F, 0x6E, 0xC0,  // movq xmm0, rax
        0x66, 0x48, 0x0F, 0x7E, 0xC1,  // movq rcx, xmm0
        0xC3,                             // ret
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto r = disp.run(0x4000, 100);
    INFO("dispatch message: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state()[ir::Gpr::Rcx] == 0xCAFEBABEDEADBEEFull);
}

TEST_CASE("e2e: PADDD xmm0, [rcx] — F2-IR-007 SSE2 memory operand") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0x66, 0x0F, 0xFE, 0x01,  // paddd xmm0, [rcx]
        0xC3,                     // ret
    };
    alignas(16) std::uint32_t mem_operand[4] = {10u, 20u, 30u, 40u};
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack4 = [](std::uint32_t a, std::uint32_t b) -> std::uint64_t {
        return static_cast<std::uint64_t>(a) |
               (static_cast<std::uint64_t>(b) << 32);
    };
    disp.state().xmm[0].lo = pack4(1u, 2u);
    disp.state().xmm[0].hi = pack4(3u, 4u);
    disp.state()[ir::Gpr::Rcx] =
        reinterpret_cast<std::uint64_t>(&mem_operand[0]);

    auto r = disp.run(0x4000, 100);
    INFO("dispatch message: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[0].lo == pack4(11u, 22u));
    REQUIRE(disp.state().xmm[0].hi == pack4(33u, 44u));
}

TEST_CASE("e2e: cache hit — running the same blob twice reuses the translation") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }

    translator::Translator tx;
    std::vector<std::uint8_t> bytes{
        0x48, 0xB8, 0x55, 0xAA, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= bytes.size()) return {};
        return std::span<const std::uint8_t>(bytes.data() + off,
                                             bytes.size() - off);
    };

    // First run: cold path.
    runtime::Dispatcher d1{tx, reader};
    d1.install_halt_return_stack();  // the RET pops the halt sentinel
    REQUIRE(d1.run(0x4000, 100).exit == runtime::DispatchExit::Halted);
    REQUIRE(tx.stats().cache_misses == 1);
    REQUIRE(tx.stats().cache_hits == 0);

    // Second run on the *same* Translator — should hit the cache.
    runtime::Dispatcher d2{tx, reader};
    d2.install_halt_return_stack();
    REQUIRE(d2.run(0x4000, 100).exit == runtime::DispatchExit::Halted);
    REQUIRE(tx.stats().cache_hits == 1);
}

TEST_CASE("e2e: VADDPS ymm2, ymm0, ymm1 — F2-IR-005 AVX-256 packed FP add") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // VEX C5 [byte1] 58 D1
    //   pp = 00 (PS, no mandatory prefix)
    //   L  = 1  (256-bit ymm)
    //   vvvv = inverted(0) = 1111
    //   R̅ = 1
    // byte1 = 1_1111_1_00 = 0xFC
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC5, 0xFC, 0x58, 0xD1,  // vaddps ymm2, ymm0, ymm1
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack2f = [](float a, float b) -> std::uint64_t {
        std::uint32_t aa, bb;
        std::memcpy(&aa, &a, 4); std::memcpy(&bb, &b, 4);
        return static_cast<std::uint64_t>(aa) | (static_cast<std::uint64_t>(bb) << 32);
    };
    // ymm0 = [1, 2, 3, 4, 5, 6, 7, 8] (low half xmm0 + high half ymm_hi[0])
    disp.state().xmm[0].lo = pack2f(1.0f, 2.0f);
    disp.state().xmm[0].hi = pack2f(3.0f, 4.0f);
    disp.state().ymm_hi[0].lo = pack2f(5.0f, 6.0f);
    disp.state().ymm_hi[0].hi = pack2f(7.0f, 8.0f);
    // ymm1 = [10, 20, 30, 40, 50, 60, 70, 80]
    disp.state().xmm[1].lo = pack2f(10.0f, 20.0f);
    disp.state().xmm[1].hi = pack2f(30.0f, 40.0f);
    disp.state().ymm_hi[1].lo = pack2f(50.0f, 60.0f);
    disp.state().ymm_hi[1].hi = pack2f(70.0f, 80.0f);
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    // ymm2 = ymm0 + ymm1 = [11, 22, 33, 44, 55, 66, 77, 88]
    REQUIRE(disp.state().xmm[2].lo    == pack2f(11.0f, 22.0f));
    REQUIRE(disp.state().xmm[2].hi    == pack2f(33.0f, 44.0f));
    REQUIRE(disp.state().ymm_hi[2].lo == pack2f(55.0f, 66.0f));
    REQUIRE(disp.state().ymm_hi[2].hi == pack2f(77.0f, 88.0f));
}

TEST_CASE("e2e: VXORPS ymm2, ymm0, ymm1 — F2-IR-005 AVX-256 FP bitwise xor") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // VEX C5 [byte1] 57 D1, pp = 00 (PS), L = 1 → byte1 = 0xFC
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC5, 0xFC, 0x57, 0xD1,  // vxorps ymm2, ymm0, ymm1
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    disp.state().xmm[0].lo    = 0xAAAAAAAAAAAAAAAAULL;
    disp.state().xmm[0].hi    = 0x5555555555555555ULL;
    disp.state().ymm_hi[0].lo = 0xFF00FF00FF00FF00ULL;
    disp.state().ymm_hi[0].hi = 0x123456789ABCDEF0ULL;
    disp.state().xmm[1].lo    = 0x0F0F0F0F0F0F0F0FULL;
    disp.state().xmm[1].hi    = 0xF0F0F0F0F0F0F0F0ULL;
    disp.state().ymm_hi[1].lo = 0x00FF00FF00FF00FFULL;
    disp.state().ymm_hi[1].hi = 0xFEDCBA9876543210ULL;
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[2].lo    == (0xAAAAAAAAAAAAAAAAULL ^ 0x0F0F0F0F0F0F0F0FULL));
    REQUIRE(disp.state().xmm[2].hi    == (0x5555555555555555ULL ^ 0xF0F0F0F0F0F0F0F0ULL));
    REQUIRE(disp.state().ymm_hi[2].lo == (0xFF00FF00FF00FF00ULL ^ 0x00FF00FF00FF00FFULL));
    REQUIRE(disp.state().ymm_hi[2].hi == (0x123456789ABCDEF0ULL ^ 0xFEDCBA9876543210ULL));
}

TEST_CASE("e2e: VPADDD ymm2, ymm0, ymm1 — F2-IR-005 AVX-256 32-bit lane add") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // VEX C5 [byte1] FE D1, pp=01 (66), L=1 → byte1 = 0xFD
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC5, 0xFD, 0xFE, 0xD1,  // vpaddd ymm2, ymm0, ymm1
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack4i = [](std::uint32_t a, std::uint32_t b) -> std::uint64_t {
        return static_cast<std::uint64_t>(a) | (static_cast<std::uint64_t>(b) << 32);
    };
    disp.state().xmm[0].lo    = pack4i(1u, 2u);
    disp.state().xmm[0].hi    = pack4i(3u, 4u);
    disp.state().ymm_hi[0].lo = pack4i(5u, 6u);
    disp.state().ymm_hi[0].hi = pack4i(7u, 8u);
    disp.state().xmm[1].lo    = pack4i(100u, 200u);
    disp.state().xmm[1].hi    = pack4i(300u, 400u);
    disp.state().ymm_hi[1].lo = pack4i(500u, 600u);
    disp.state().ymm_hi[1].hi = pack4i(700u, 800u);
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[2].lo    == pack4i(101u, 202u));
    REQUIRE(disp.state().xmm[2].hi    == pack4i(303u, 404u));
    REQUIRE(disp.state().ymm_hi[2].lo == pack4i(505u, 606u));
    REQUIRE(disp.state().ymm_hi[2].hi == pack4i(707u, 808u));
}

TEST_CASE("e2e: VPCMPEQD ymm2, ymm0, ymm1 — F2-IR-005 AVX-256 32-bit lane equality") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // VEX C5 [byte1] 76 D1, pp=01 (66), L=1 → byte1 = 0xFD
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC5, 0xFD, 0x76, 0xD1,  // vpcmpeqd ymm2, ymm0, ymm1
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack4i = [](std::uint32_t a, std::uint32_t b) -> std::uint64_t {
        return static_cast<std::uint64_t>(a) | (static_cast<std::uint64_t>(b) << 32);
    };
    // ymm0: lanes 0,1,2,3,4,5,6,7. ymm1: lanes 0,99,2,99,4,99,99,7.
    disp.state().xmm[0].lo    = pack4i(0u, 1u);
    disp.state().xmm[0].hi    = pack4i(2u, 3u);
    disp.state().ymm_hi[0].lo = pack4i(4u, 5u);
    disp.state().ymm_hi[0].hi = pack4i(6u, 7u);
    disp.state().xmm[1].lo    = pack4i(0u, 99u);
    disp.state().xmm[1].hi    = pack4i(2u, 99u);
    disp.state().ymm_hi[1].lo = pack4i(4u, 99u);
    disp.state().ymm_hi[1].hi = pack4i(99u, 7u);
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    // Equal lanes get all-1s (0xFFFFFFFF), unequal get 0.
    REQUIRE(disp.state().xmm[2].lo    == pack4i(0xFFFFFFFFu, 0u));
    REQUIRE(disp.state().xmm[2].hi    == pack4i(0xFFFFFFFFu, 0u));
    REQUIRE(disp.state().ymm_hi[2].lo == pack4i(0xFFFFFFFFu, 0u));
    REQUIRE(disp.state().ymm_hi[2].hi == pack4i(0u, 0xFFFFFFFFu));
}

TEST_CASE("e2e: VUNPCKLPS ymm2, ymm0, ymm1 — F2-IR-005 AVX-256 FP lane interleave") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // VEX C5 [byte1] 14 D1, pp=00 (PS), L=1 → byte1 = 0xFC
    // VUNPCKLPS interleaves the LOW two lanes of each 128-bit half:
    //   ymm2[0] = ymm0[0]; ymm2[1] = ymm1[0]; ymm2[2] = ymm0[1]; ymm2[3] = ymm1[1]
    //   ymm2[4] = ymm0[4]; ymm2[5] = ymm1[4]; ymm2[6] = ymm0[5]; ymm2[7] = ymm1[5]
    // (Per-128-bit-lane semantics — no cross-lane.)
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC5, 0xFC, 0x14, 0xD1,  // vunpcklps ymm2, ymm0, ymm1
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack4i = [](std::uint32_t a, std::uint32_t b) -> std::uint64_t {
        return static_cast<std::uint64_t>(a) | (static_cast<std::uint64_t>(b) << 32);
    };
    // ymm0: 1,2,3,4 | 5,6,7,8     ymm1: 11,12,13,14 | 15,16,17,18
    disp.state().xmm[0].lo    = pack4i(1u, 2u);
    disp.state().xmm[0].hi    = pack4i(3u, 4u);
    disp.state().ymm_hi[0].lo = pack4i(5u, 6u);
    disp.state().ymm_hi[0].hi = pack4i(7u, 8u);
    disp.state().xmm[1].lo    = pack4i(11u, 12u);
    disp.state().xmm[1].hi    = pack4i(13u, 14u);
    disp.state().ymm_hi[1].lo = pack4i(15u, 16u);
    disp.state().ymm_hi[1].hi = pack4i(17u, 18u);
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    // Expected ymm2 lanes: 1,11,2,12 | 5,15,6,16
    REQUIRE(disp.state().xmm[2].lo    == pack4i(1u, 11u));
    REQUIRE(disp.state().xmm[2].hi    == pack4i(2u, 12u));
    REQUIRE(disp.state().ymm_hi[2].lo == pack4i(5u, 15u));
    REQUIRE(disp.state().ymm_hi[2].hi == pack4i(6u, 16u));
}

TEST_CASE("e2e: VFMADD231PS xmm2, xmm0, xmm1 — F2-IR-006 fused multiply-add") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // VEX C4 [byte1 byte2] B8 D1
    //   C4 form because FMA needs the W-bit slot:
    //   byte1: R̅ X̅ B̅ mmmmm = 1_1_1_00010 = 0xE2
    //          (mmmmm = 0b00010 = 2 → 0F 38 escape)
    //   byte2: W vvvv L pp     = 0_1111_0_01 = 0x79
    //          (W=0 → PS; vvvv = inverted(xmm0=0) = 1111; L=0; pp=01)
    // VFMADD231PS xmm2, xmm0, xmm1 → xmm2 = (xmm0 * xmm1) + xmm2
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC4, 0xE2, 0x79, 0xB8, 0xD1,
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack2f = [](float a, float b) -> std::uint64_t {
        std::uint32_t aa, bb;
        std::memcpy(&aa, &a, 4); std::memcpy(&bb, &b, 4);
        return static_cast<std::uint64_t>(aa) | (static_cast<std::uint64_t>(bb) << 32);
    };
    // xmm0 = [2, 3, 4, 5]   xmm1 = [10, 20, 30, 40]   xmm2 = [1, 1, 1, 1]
    // result xmm2 = (xmm0*xmm1) + xmm2 = [21, 61, 121, 201]
    disp.state().xmm[0].lo = pack2f(2.0f, 3.0f);
    disp.state().xmm[0].hi = pack2f(4.0f, 5.0f);
    disp.state().xmm[1].lo = pack2f(10.0f, 20.0f);
    disp.state().xmm[1].hi = pack2f(30.0f, 40.0f);
    disp.state().xmm[2].lo = pack2f(1.0f, 1.0f);
    disp.state().xmm[2].hi = pack2f(1.0f, 1.0f);
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[2].lo == pack2f(21.0f, 61.0f));
    REQUIRE(disp.state().xmm[2].hi == pack2f(121.0f, 201.0f));
}

TEST_CASE("e2e: VFMSUB231PS xmm2, xmm0, xmm1 — F2-IR-006 b*c - a") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // VFMSUB231PS: xmm2 = (xmm0 * xmm1) - xmm2
    // Opcode 0xBA (231 PS = MSUB packed). C4 byte2 = 0x79 (W=0, vvvv=0xF, L=0, pp=01).
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC4, 0xE2, 0x79, 0xBA, 0xD1,
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack2f = [](float a, float b) -> std::uint64_t {
        std::uint32_t aa, bb;
        std::memcpy(&aa, &a, 4); std::memcpy(&bb, &b, 4);
        return static_cast<std::uint64_t>(aa) | (static_cast<std::uint64_t>(bb) << 32);
    };
    // xmm0 = [2, 3, 4, 5]   xmm1 = [10, 20, 30, 40]   xmm2 = [1, 1, 1, 1]
    // result = (xmm0*xmm1) - xmm2 = [19, 59, 119, 199]
    disp.state().xmm[0].lo = pack2f(2.0f, 3.0f);
    disp.state().xmm[0].hi = pack2f(4.0f, 5.0f);
    disp.state().xmm[1].lo = pack2f(10.0f, 20.0f);
    disp.state().xmm[1].hi = pack2f(30.0f, 40.0f);
    disp.state().xmm[2].lo = pack2f(1.0f, 1.0f);
    disp.state().xmm[2].hi = pack2f(1.0f, 1.0f);
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[2].lo == pack2f(19.0f, 59.0f));
    REQUIRE(disp.state().xmm[2].hi == pack2f(119.0f, 199.0f));
}

TEST_CASE("e2e: VINSERTF128 ymm0, ymm1, xmm2, 1 — F2-IR-005 high-lane insert") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // C4 byte1 byte2 18 ModRM imm8
    //   byte1: R̅ X̅ B̅ mmmmm = 1_1_1_00011 = 0xE3 (mmmmm=3 → 0F 3A)
    //   byte2: W=0 vvvv=inv(1)=1110 L=1 pp=01 → 0_1110_1_01 = 0x75
    //   ModRM C2 = 11_000_010 → reg=ymm0, rm=xmm2
    //   imm8 = 0x01 → insert into HIGH half
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC4, 0xE3, 0x75, 0x18, 0xC2, 0x01,
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    // ymm1.lo = [11 12], ymm1.hi = [13 14] (256-bit) — survives unchanged in ymm0.lo.
    // xmm2 = [99 100] — inserted into ymm0.hi (replacing the original ymm1.hi).
    disp.state().xmm[1].lo    = 0x000000010000000BULL;  // arbitrary
    disp.state().xmm[1].hi    = 0x000000020000000CULL;
    disp.state().ymm_hi[1].lo = 0xDEADBEEF00000000ULL;  // overwritten by xmm2
    disp.state().ymm_hi[1].hi = 0xCAFEBABE00000000ULL;  // overwritten by xmm2
    disp.state().xmm[2].lo    = 0x1111111122222222ULL;
    disp.state().xmm[2].hi    = 0x3333333344444444ULL;
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    // ymm0 low half = ymm1 low half (preserved from vvvv).
    REQUIRE(disp.state().xmm[0].lo == 0x000000010000000BULL);
    REQUIRE(disp.state().xmm[0].hi == 0x000000020000000CULL);
    // ymm0 high half = xmm2 (the 128-bit source).
    REQUIRE(disp.state().ymm_hi[0].lo == 0x1111111122222222ULL);
    REQUIRE(disp.state().ymm_hi[0].hi == 0x3333333344444444ULL);
}

TEST_CASE("e2e: VEXTRACTF128 xmm0, ymm1, 1 — F2-IR-005 high-lane extract") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // C4 byte1 byte2 19 ModRM imm8
    //   byte1 = 0xE3 (mmmmm=3)
    //   byte2: W=0 vvvv=1111 L=1 pp=01 → 0_1111_1_01 = 0x7D
    //   ModRM C8 = 11_001_000 → reg=ymm1 (source), rm=xmm0 (dst)
    //   imm8 = 0x01 → extract HIGH half
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC4, 0xE3, 0x7D, 0x19, 0xC8, 0x01,
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    disp.state().xmm[1].lo    = 0xAAAAAAAAAAAAAAAAULL;
    disp.state().xmm[1].hi    = 0xBBBBBBBBBBBBBBBBULL;
    disp.state().ymm_hi[1].lo = 0xCCCCCCCCCCCCCCCCULL;
    disp.state().ymm_hi[1].hi = 0xDDDDDDDDDDDDDDDDULL;
    // Pre-fill xmm0's upper half (ymm_hi[0]) with garbage so we can confirm it gets zeroed.
    disp.state().ymm_hi[0].lo = 0xEEEEEEEEEEEEEEEEULL;
    disp.state().ymm_hi[0].hi = 0xFFFFFFFFFFFFFFFFULL;
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    // xmm0 = ymm1 high half.
    REQUIRE(disp.state().xmm[0].lo == 0xCCCCCCCCCCCCCCCCULL);
    REQUIRE(disp.state().xmm[0].hi == 0xDDDDDDDDDDDDDDDDULL);
    // ymm_hi[0] is zeroed (xmm-form extract zeroes the upper half).
    REQUIRE(disp.state().ymm_hi[0].lo == 0ULL);
    REQUIRE(disp.state().ymm_hi[0].hi == 0ULL);
}

TEST_CASE("e2e: VPERM2F128 ymm0, ymm1, ymm2, 0x21 — F2-IR-005 lane swap") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // imm8 = 0x21 → low half ← src.lo (sel=0b01=ymm1.hi), high half ← src2.lo (sel=0b10=ymm2.lo)
    //   bits [1:0] = 01 → vvvv.hi (ymm1.hi) for low half
    //   bits [5:4] = 10 → src.lo (ymm2.lo) for high half
    // C4 byte1 byte2 06 ModRM imm8
    //   byte1 = 0xE3, byte2: W=0 vvvv=inv(1)=1110 L=1 pp=01 → 0x75
    //   ModRM C2 = 11_000_010 → reg=ymm0, rm=ymm2
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC4, 0xE3, 0x75, 0x06, 0xC2, 0x21,
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    disp.state().xmm[1].lo    = 0x1111111111111111ULL;  // ymm1.lo
    disp.state().xmm[1].hi    = 0x2222222222222222ULL;
    disp.state().ymm_hi[1].lo = 0x3333333333333333ULL;  // ymm1.hi (→ ymm0.lo)
    disp.state().ymm_hi[1].hi = 0x4444444444444444ULL;
    disp.state().xmm[2].lo    = 0xAAAAAAAAAAAAAAAAULL;  // ymm2.lo (→ ymm0.hi)
    disp.state().xmm[2].hi    = 0xBBBBBBBBBBBBBBBBULL;
    disp.state().ymm_hi[2].lo = 0xCCCCCCCCCCCCCCCCULL;
    disp.state().ymm_hi[2].hi = 0xDDDDDDDDDDDDDDDDULL;
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    // ymm0.lo = ymm1.hi
    REQUIRE(disp.state().xmm[0].lo == 0x3333333333333333ULL);
    REQUIRE(disp.state().xmm[0].hi == 0x4444444444444444ULL);
    // ymm0.hi = ymm2.lo
    REQUIRE(disp.state().ymm_hi[0].lo == 0xAAAAAAAAAAAAAAAAULL);
    REQUIRE(disp.state().ymm_hi[0].hi == 0xBBBBBBBBBBBBBBBBULL);
}

TEST_CASE("e2e: VPERM2F128 ymm0, ymm1, ymm2, 0x88 — F2-IR-005 zero both halves") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // imm8 = 0x88 → bit 3 set (zero low) and bit 7 set (zero high) → all zero.
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC4, 0xE3, 0x75, 0x06, 0xC2, 0x88,
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    disp.state().xmm[1].lo    = 0x1111111111111111ULL;
    disp.state().xmm[1].hi    = 0x2222222222222222ULL;
    disp.state().ymm_hi[1].lo = 0x3333333333333333ULL;
    disp.state().ymm_hi[1].hi = 0x4444444444444444ULL;
    disp.state().xmm[2].lo    = 0xAAAAAAAAAAAAAAAAULL;
    disp.state().xmm[2].hi    = 0xBBBBBBBBBBBBBBBBULL;
    disp.state().ymm_hi[2].lo = 0xCCCCCCCCCCCCCCCCULL;
    disp.state().ymm_hi[2].hi = 0xDDDDDDDDDDDDDDDDULL;
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[0].lo    == 0ULL);
    REQUIRE(disp.state().xmm[0].hi    == 0ULL);
    REQUIRE(disp.state().ymm_hi[0].lo == 0ULL);
    REQUIRE(disp.state().ymm_hi[0].hi == 0ULL);
}

TEST_CASE("e2e: VBROADCASTSS ymm0, xmm1 — F2-IR-005 lane-0 broadcast to all 8") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // VEX C4 byte1 byte2 18 C1
    //   byte1 = 0xE2 (R̅=1 X̅=1 B̅=1 mmmmm=2)
    //   byte2 = 0x7D (W=0 vvvv=0xF L=1 pp=01)
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC4, 0xE2, 0x7D, 0x18, 0xC1,
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack2f = [](float a, float b) -> std::uint64_t {
        std::uint32_t aa, bb;
        std::memcpy(&aa, &a, 4); std::memcpy(&bb, &b, 4);
        return static_cast<std::uint64_t>(aa) | (static_cast<std::uint64_t>(bb) << 32);
    };
    // xmm1 lane 0 = 7.5; rest of xmm1 / ymm_hi[1] are noise.
    disp.state().xmm[1].lo = pack2f(7.5f, -1.0f);
    disp.state().xmm[1].hi = pack2f(-2.0f, -3.0f);
    disp.state().ymm_hi[1].lo = pack2f(-4.0f, -5.0f);
    disp.state().ymm_hi[1].hi = pack2f(-6.0f, -7.0f);
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    // ymm0 = [7.5] × 8.
    const std::uint64_t pat = pack2f(7.5f, 7.5f);
    REQUIRE(disp.state().xmm[0].lo    == pat);
    REQUIRE(disp.state().xmm[0].hi    == pat);
    REQUIRE(disp.state().ymm_hi[0].lo == pat);
    REQUIRE(disp.state().ymm_hi[0].hi == pat);
}

TEST_CASE("e2e: VBROADCASTSD ymm0, xmm1 — F2-IR-005 64-bit broadcast to all 4") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // C4 byte1 byte2 19 C1, byte2 = 0x7D
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC4, 0xE2, 0x7D, 0x19, 0xC1,
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto u64_of_double = [](double v) -> std::uint64_t {
        std::uint64_t bits; std::memcpy(&bits, &v, 8); return bits;
    };
    disp.state().xmm[1].lo    = u64_of_double(3.14);
    disp.state().xmm[1].hi    = u64_of_double(-99.0);   // ignored
    disp.state().ymm_hi[1].lo = u64_of_double(-99.0);   // ignored
    disp.state().ymm_hi[1].hi = u64_of_double(-99.0);   // ignored
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    const std::uint64_t pat = u64_of_double(3.14);
    REQUIRE(disp.state().xmm[0].lo    == pat);
    REQUIRE(disp.state().xmm[0].hi    == pat);
    REQUIRE(disp.state().ymm_hi[0].lo == pat);
    REQUIRE(disp.state().ymm_hi[0].hi == pat);
}

TEST_CASE("e2e: REP STOSB — F2-BK-008 native loop memset") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // F3 AA → REP STOSB. Fills [RDI..RDI+RCX) with AL bytes.
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xF3, 0xAA,
        0xC3,
    };
    alignas(16) static std::uint8_t buf[80] = {};
    for (auto& b : buf) b = 0xAA;
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rax)] = 0x55ULL;
    disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rcx)] = 32ULL;
    disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rdi)] =
        reinterpret_cast<std::uint64_t>(&buf[8]);
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    for (int i = 0; i < 8; ++i)  REQUIRE(buf[i] == 0xAA);
    for (int i = 8; i < 8 + 32; ++i) REQUIRE(buf[i] == 0x55);
    for (int i = 8 + 32; i < 80; ++i) REQUIRE(buf[i] == 0xAA);
    REQUIRE(disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rcx)] == 0ULL);
    REQUIRE(disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rdi)] ==
            reinterpret_cast<std::uint64_t>(&buf[8 + 32]));
}

TEST_CASE("e2e: REP MOVSB — F2-BK-009 native loop memcpy") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xF3, 0xA4,
        0xC3,
    };
    alignas(16) static std::uint8_t src_buf[64] = {};
    alignas(16) static std::uint8_t dst_buf[64] = {};
    for (std::size_t i = 0; i < 64; ++i) {
        src_buf[i] = static_cast<std::uint8_t>(i + 1);
        dst_buf[i] = 0xCC;
    }
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rcx)] = 24ULL;
    disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rsi)] =
        reinterpret_cast<std::uint64_t>(&src_buf[0]);
    disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rdi)] =
        reinterpret_cast<std::uint64_t>(&dst_buf[0]);
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    for (std::size_t i = 0; i < 24; ++i) REQUIRE(dst_buf[i] == src_buf[i]);
    for (std::size_t i = 24; i < 64; ++i) REQUIRE(dst_buf[i] == 0xCC);
    REQUIRE(disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rcx)] == 0ULL);
    REQUIRE(r.stats.direct_thread_misses == 1u);
    REQUIRE(r.stats.direct_thread_installs == 1u);
}

TEST_CASE("e2e: REP STOSB with RCX=0 — F2-BK-008 zero-iteration skip") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xF3, 0xAA,
        0xC3,
    };
    alignas(16) static std::uint8_t buf[16] = {};
    for (auto& b : buf) b = 0xAA;
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rax)] = 0x99ULL;
    disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rcx)] = 0ULL;
    disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rdi)] =
        reinterpret_cast<std::uint64_t>(&buf[0]);
    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    for (auto b : buf) REQUIRE(b == 0xAA);
}

// F2-IR-054 — real CALL/RET round-trip. With `set_real_call_ret(true)`,
// CALL pushes the next_pc onto the guest stack and jumps to the
// callee; the callee's RET pops the saved next_pc and JumpReg's back
// to it. The test sets up RSP pointing at a 16-byte buffer whose
// "below RSP" cell holds 0 — so the caller's own RET (after the
// CALL returns) pops 0, which the dispatcher recognises as the halt
// sentinel and stops cleanly.
TEST_CASE("e2e: real CALL/RET round-trip — F2-IR-054 opt-in semantics") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    tx.set_real_call_ret(true);
    // Layout (PC = 0x4000 base):
    //   0x4000:  B8 11 00 00 00       MOV EAX, 0x11      ; pre-CALL marker
    //   0x4005:  E8 06 00 00 00       CALL 0x4010
    //   0x400A:  B8 22 00 00 00       MOV EAX, 0x22      ; post-CALL marker
    //   0x400F:  C3                   RET                ; halt via RSP=0 sentinel
    //   0x4010:  B8 33 00 00 00       MOV EAX, 0x33      ; in callee
    //   0x4015:  C3                   RET                ; pops 0x400A, returns
    std::vector<std::uint8_t> code{
        0xB8, 0x11, 0x00, 0x00, 0x00,         // 0x4000
        0xE8, 0x06, 0x00, 0x00, 0x00,         // 0x4005 → call 0x4010
        0xB8, 0x22, 0x00, 0x00, 0x00,         // 0x400A
        0xC3,                                   // 0x400F
        0xB8, 0x33, 0x00, 0x00, 0x00,         // 0x4010
        0xC3,                                   // 0x4015 → ret to 0x400A
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    // The helper sets up a 16-slot internal halt-return stack and
    // points RSP at the top slot. The caller's outermost RET (at
    // 0x400F) will pop a 0 from one of the lower slots → halt
    // sentinel.
    disp.install_halt_return_stack();
    auto r = disp.run(0x4000, /*max_steps=*/32);
    INFO("dispatch: " << r.message
                      << "  steps=" << r.stats.steps_taken);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    // Final EAX should be 0x22 — the post-CALL marker, executed
    // *after* the callee's RET resumed at 0x400A.
    REQUIRE((disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rax)] & 0xFFFFFFFFull)
            == 0x22ull);
}

// Blocker A adversarial test: REP STOSB with RCX strictly greater than
// the per-call clamp must finish correctly across multiple dispatcher
// hops, not hang the host. RCX = kRepMaxBytesPerCall + small_overflow
// triggers exactly two REP blocks (one clamped, one finisher) plus the
// trailing RET block — 3 dispatcher steps total.
TEST_CASE("e2e: REP STOSB beyond clamp — Blocker A re-entry path") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    constexpr std::uint64_t kIterCap  = ir::kRepMaxBytesPerCall;  // I8: 16 MiB
    constexpr std::uint64_t kOverflow = 7u;
    constexpr std::uint64_t kCount    = kIterCap + kOverflow;
    // Heap buffer with one-byte sentinels at both ends.
    std::vector<std::uint8_t> buf(kCount + 16u, 0xAAu);
    translator::Translator tx;
    std::vector<std::uint8_t> code{0xF3, 0xAA, 0xC3};   // REP STOSB; RET
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rax)] = 0x42ULL;
    disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rcx)] = kCount;
    disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rdi)] =
        reinterpret_cast<std::uint64_t>(&buf[8]);
    auto r = disp.run(0x4000, /*max_steps=*/8);
    INFO("dispatch: " << r.message
                      << "  steps=" << r.stats.steps_taken);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    // Two REP blocks (clamp + finisher) + one RET block = 3 dispatcher
    // hops. If the clamp were missing this would either hang or finish
    // in 1 hop with 16 MiB stored in one host-side burst.
    REQUIRE(r.stats.steps_taken == 3u);
    // Pre/post-buffer sentinels intact.
    REQUIRE(buf[7]                      == 0xAAu);
    REQUIRE(buf[8u + kCount]            == 0xAAu);
    // Body fully filled with 0x42.
    REQUIRE(buf[8u]                     == 0x42u);
    REQUIRE(buf[8u + kCount - 1u]       == 0x42u);
    REQUIRE(buf[8u + kIterCap]          == 0x42u);   // boundary byte
    REQUIRE(buf[8u + kIterCap - 1u]     == 0x42u);
    // Guest state at halt.
    REQUIRE(disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rcx)] == 0ULL);
    REQUIRE(disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rdi)] ==
            reinterpret_cast<std::uint64_t>(&buf[8u + kCount]));
    REQUIRE(disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rax)] == 0x42ULL);
    REQUIRE(r.stats.direct_thread_hits == 1u);
    REQUIRE(r.stats.direct_thread_misses == 1u);
    REQUIRE(r.stats.direct_thread_installs == 1u);
}

TEST_CASE("e2e: VPGATHERDD xmm1, [rax + xmm2*4], xmm3 — F2-IR-059 masked gather") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }

    translator::Translator tx;
    std::vector<std::uint8_t> bytes{
        0xC4, 0xE2, 0x61, 0x90, 0x0C, 0x90,  // vpgatherdd xmm1,[rax+xmm2*4],xmm3
        0xC3,                                 // ret
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= bytes.size()) return {};
        return std::span<const std::uint8_t>(bytes.data() + off,
                                             bytes.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();

    alignas(16) std::array<std::uint32_t, 8> table{
        100u, 101u, 102u, 103u, 104u, 105u, 106u, 107u};
    disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rax)] =
        reinterpret_cast<std::uint64_t>(table.data());
    // Indices per dword lane: {7, POISON, 3, POISON}. The poisoned
    // lanes are masked off below — a correct gather never forms or
    // dereferences their addresses (they point ~4 GiB away).
    disp.state().xmm[2].lo = (0x4000'0000ull << 32) | 7ull;
    disp.state().xmm[2].hi = (0x4000'0000ull << 32) | 3ull;
    // Mask MSBs: lanes 0 and 2 active, lanes 1 and 3 inactive.
    disp.state().xmm[3].lo = 0x0000'0000'8000'0000ull;
    disp.state().xmm[3].hi = 0x0000'0000'8000'0000ull;
    // Pre-fill the destination so kept lanes are observable.
    disp.state().xmm[1].lo = 0xAAAA'AAAA'BBBB'BBBBull;
    disp.state().xmm[1].hi = 0xCCCC'CCCC'DDDD'DDDDull;

    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    // Lane 0 ← table[7], lane 1 keeps 0xAAAAAAAA,
    // lane 2 ← table[3], lane 3 keeps 0xCCCCCCCC.
    REQUIRE(disp.state().xmm[1].lo == ((0xAAAA'AAAAull << 32) | 107ull));
    REQUIRE(disp.state().xmm[1].hi == ((0xCCCC'CCCCull << 32) | 103ull));
    // The mask register reads as all zeroes on completion, and the
    // VEX.128 writes cleared both upper lanes.
    REQUIRE(disp.state().xmm[3].lo == 0u);
    REQUIRE(disp.state().xmm[3].hi == 0u);
    REQUIRE(disp.state().ymm_hi[1].lo == 0u);
    REQUIRE(disp.state().ymm_hi[3].lo == 0u);
}

TEST_CASE("e2e: VPGATHERDQ xmm1, [rax + xmm2*8], xmm3 — F2-IR-059 qword gather") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }

    translator::Translator tx;
    std::vector<std::uint8_t> bytes{
        0xC4, 0xE2, 0xE1, 0x90, 0x0C, 0xD0,  // vpgatherdq xmm1,[rax+xmm2*8],xmm3
        0xC3,                                 // ret
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= bytes.size()) return {};
        return std::span<const std::uint8_t>(bytes.data() + off,
                                             bytes.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();

    alignas(16) std::array<std::uint64_t, 4> table{
        1000ull, 1001ull, 1002ull, 1003ull};
    disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rax)] =
        reinterpret_cast<std::uint64_t>(table.data());
    // Dword indices for the two qword elements: {3, POISON}. The
    // poisoned index belongs to the masked-off element 1.
    disp.state().xmm[2].lo = (0x4000'0000ull << 32) | 3ull;
    disp.state().xmm[2].hi = 0u;
    // Qword mask lanes: element 0 active (bit 63), element 1 inactive.
    disp.state().xmm[3].lo = 0x8000'0000'0000'0000ull;
    disp.state().xmm[3].hi = 0x7FFF'FFFF'FFFF'FFFFull;  // MSB clear
    disp.state().xmm[1].lo = 0xAAAA'AAAA'BBBB'BBBBull;
    disp.state().xmm[1].hi = 0xCCCC'CCCC'DDDD'DDDDull;

    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[1].lo == 1003ull);              // table[3]
    REQUIRE(disp.state().xmm[1].hi == 0xCCCC'CCCC'DDDD'DDDDull);
    REQUIRE(disp.state().xmm[3].lo == 0u);
    REQUIRE(disp.state().xmm[3].hi == 0u);
    REQUIRE(disp.state().ymm_hi[1].lo == 0u);
}

TEST_CASE("e2e: VPGATHERQQ xmm1, [rax + xmm2*8], xmm3 — F2-IR-059 qword indices") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }

    translator::Translator tx;
    std::vector<std::uint8_t> bytes{
        0xC4, 0xE2, 0xE1, 0x91, 0x0C, 0xD0,  // vpgatherqq xmm1,[rax+xmm2*8],xmm3
        0xC3,                                 // ret
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= bytes.size()) return {};
        return std::span<const std::uint8_t>(bytes.data() + off,
                                             bytes.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();

    alignas(16) std::array<std::uint64_t, 4> table{
        2000ull, 2001ull, 2002ull, 2003ull};
    disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rax)] =
        reinterpret_cast<std::uint64_t>(table.data());
    // Qword indices: element 0 gathers table[2]; element 1 is a
    // poisoned 64-bit index on a masked-off lane.
    disp.state().xmm[2].lo = 2ull;
    disp.state().xmm[2].hi = 0x4000'0000'0000'0000ull;
    disp.state().xmm[3].lo = 0x8000'0000'0000'0000ull;
    disp.state().xmm[3].hi = 0u;
    disp.state().xmm[1].lo = 0xAAAA'AAAA'BBBB'BBBBull;
    disp.state().xmm[1].hi = 0xCCCC'CCCC'DDDD'DDDDull;

    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[1].lo == 2002ull);              // table[2]
    REQUIRE(disp.state().xmm[1].hi == 0xCCCC'CCCC'DDDD'DDDDull);
    REQUIRE(disp.state().xmm[3].lo == 0u);
    REQUIRE(disp.state().xmm[3].hi == 0u);
    REQUIRE(disp.state().ymm_hi[1].lo == 0u);
}

TEST_CASE("e2e: VPGATHERQD xmm1, [rax + xmm2*4], xmm3 — F2-IR-059 mixed widths") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }

    translator::Translator tx;
    std::vector<std::uint8_t> bytes{
        0xC4, 0xE2, 0x61, 0x91, 0x0C, 0x90,  // vpgatherqd xmm1,[rax+xmm2*4],xmm3
        0xC3,                                 // ret
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= bytes.size()) return {};
        return std::span<const std::uint8_t>(bytes.data() + off,
                                             bytes.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();

    alignas(16) std::array<std::uint32_t, 8> table{
        100u, 101u, 102u, 103u, 104u, 105u, 106u, 107u};
    disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rax)] =
        reinterpret_cast<std::uint64_t>(table.data());
    // Qword indices for the two dword elements: {5, POISON(masked)}.
    disp.state().xmm[2].lo = 5ull;
    disp.state().xmm[2].hi = 0x4000'0000'0000'0000ull;
    // Dword mask lanes 0..1: lane 0 active, lane 1 inactive.
    disp.state().xmm[3].lo = 0x0000'0000'8000'0000ull;
    disp.state().xmm[3].hi = 0u;
    disp.state().xmm[1].lo = 0xAAAA'AAAA'BBBB'BBBBull;
    disp.state().xmm[1].hi = 0xCCCC'CCCC'DDDD'DDDDull;

    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    // Lane 0 ← table[5]; lane 1 keeps 0xAAAAAAAA; the destination's
    // upper 64 bits read as zero for the QD form.
    REQUIRE(disp.state().xmm[1].lo == ((0xAAAA'AAAAull << 32) | 105ull));
    REQUIRE(disp.state().xmm[1].hi == 0u);
    REQUIRE(disp.state().xmm[3].lo == 0u);
    REQUIRE(disp.state().xmm[3].hi == 0u);
    REQUIRE(disp.state().ymm_hi[1].lo == 0u);
}

TEST_CASE("e2e: VGATHERDPS xmm1, [rax + xmm2*4], xmm3 — F2-IR-059 FP gather") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }

    translator::Translator tx;
    std::vector<std::uint8_t> bytes{
        0xC4, 0xE2, 0x61, 0x92, 0x0C, 0x90,  // vgatherdps xmm1,[rax+xmm2*4],xmm3
        0xC3,                                 // ret
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= bytes.size()) return {};
        return std::span<const std::uint8_t>(bytes.data() + off,
                                             bytes.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();

    // Float bit patterns (the gather is a bit copy; 0x3FC00000=1.5f).
    alignas(16) std::array<std::uint32_t, 8> table{
        0x3F80'0000u, 0x3FC0'0000u, 0x4000'0000u, 0x4040'0000u,
        0x4080'0000u, 0x40A0'0000u, 0x40C0'0000u, 0x40E0'0000u};
    disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rax)] =
        reinterpret_cast<std::uint64_t>(table.data());
    disp.state().xmm[2].lo = (0x4000'0000ull << 32) | 6ull;
    disp.state().xmm[2].hi = (0x4000'0000ull << 32) | 1ull;
    // Mask = float sign bit per lane: lanes 0 and 2 active.
    disp.state().xmm[3].lo = 0x0000'0000'8000'0000ull;
    disp.state().xmm[3].hi = 0x0000'0000'8000'0000ull;
    disp.state().xmm[1].lo = 0xAAAA'AAAA'BBBB'BBBBull;
    disp.state().xmm[1].hi = 0xCCCC'CCCC'DDDD'DDDDull;

    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[1].lo ==
            ((0xAAAA'AAAAull << 32) | 0x40C0'0000ull));      // table[6]
    REQUIRE(disp.state().xmm[1].hi ==
            ((0xCCCC'CCCCull << 32) | 0x3FC0'0000ull));      // table[1]
    REQUIRE(disp.state().xmm[3].lo == 0u);
    REQUIRE(disp.state().xmm[3].hi == 0u);
    REQUIRE(disp.state().ymm_hi[1].lo == 0u);
}

TEST_CASE("e2e: VPGATHERDD ymm1, [rax + ymm2*4], ymm3 — F2-IR-059 ymm gather") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }

    translator::Translator tx;
    std::vector<std::uint8_t> bytes{
        0xC4, 0xE2, 0x65, 0x90, 0x0C, 0x90,  // vpgatherdd ymm1,[rax+ymm2*4],ymm3
        0xC3,                                 // ret
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= bytes.size()) return {};
        return std::span<const std::uint8_t>(bytes.data() + off,
                                             bytes.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();

    alignas(16) std::array<std::uint32_t, 8> table{
        100u, 101u, 102u, 103u, 104u, 105u, 106u, 107u};
    disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rax)] =
        reinterpret_cast<std::uint64_t>(table.data());
    // Indices: lo half {7, P, 3, P}, hi half {1, P, 5, P} with the
    // poisoned lanes masked off in BOTH halves.
    disp.state().xmm[2].lo    = (0x4000'0000ull << 32) | 7ull;
    disp.state().xmm[2].hi    = (0x4000'0000ull << 32) | 3ull;
    disp.state().ymm_hi[2].lo = (0x4000'0000ull << 32) | 1ull;
    disp.state().ymm_hi[2].hi = (0x4000'0000ull << 32) | 5ull;
    // Mask MSBs: lanes 0 and 2 of each half active.
    disp.state().xmm[3].lo    = 0x0000'0000'8000'0000ull;
    disp.state().xmm[3].hi    = 0x0000'0000'8000'0000ull;
    disp.state().ymm_hi[3].lo = 0x0000'0000'8000'0000ull;
    disp.state().ymm_hi[3].hi = 0x0000'0000'8000'0000ull;
    disp.state().xmm[1].lo    = 0xAAAA'AAAA'BBBB'BBBBull;
    disp.state().xmm[1].hi    = 0xCCCC'CCCC'DDDD'DDDDull;
    disp.state().ymm_hi[1].lo = 0xEEEE'EEEE'FFFF'FFFFull;
    disp.state().ymm_hi[1].hi = 0x1111'1111'2222'2222ull;

    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    // Lo half: lane 0 ← table[7], lane 2 ← table[3], odd lanes kept.
    REQUIRE(disp.state().xmm[1].lo == ((0xAAAA'AAAAull << 32) | 107ull));
    REQUIRE(disp.state().xmm[1].hi == ((0xCCCC'CCCCull << 32) | 103ull));
    // Hi half: lane 4 ← table[1], lane 6 ← table[5], odd lanes kept.
    REQUIRE(disp.state().ymm_hi[1].lo ==
            ((0xEEEE'EEEEull << 32) | 101ull));
    REQUIRE(disp.state().ymm_hi[1].hi ==
            ((0x1111'1111ull << 32) | 105ull));
    // Full ymm mask cleared.
    REQUIRE(disp.state().xmm[3].lo == 0u);
    REQUIRE(disp.state().xmm[3].hi == 0u);
    REQUIRE(disp.state().ymm_hi[3].lo == 0u);
    REQUIRE(disp.state().ymm_hi[3].hi == 0u);
}

TEST_CASE("e2e: VPGATHERDQ ymm1, [rax + xmm2*8], ymm3 — F2-IR-059 split index") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }

    translator::Translator tx;
    std::vector<std::uint8_t> bytes{
        0xC4, 0xE2, 0xE5, 0x90, 0x0C, 0xD0,  // vpgatherdq ymm1,[rax+xmm2*8],ymm3
        0xC3,                                 // ret
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= bytes.size()) return {};
        return std::span<const std::uint8_t>(bytes.data() + off,
                                             bytes.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();

    alignas(16) std::array<std::uint64_t, 4> table{
        1000ull, 1001ull, 1002ull, 1003ull};
    disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rax)] =
        reinterpret_cast<std::uint64_t>(table.data());
    // One xmm of dword indices feeds all four qword elements; index
    // lanes 2..3 drive the hi half (index_lane_base = 2 lowering).
    // Lanes: {3, P, 1, P} with the poisoned ones masked off.
    disp.state().xmm[2].lo = (0x4000'0000ull << 32) | 3ull;
    disp.state().xmm[2].hi = (0x4000'0000ull << 32) | 1ull;
    // Qword mask lanes {on, off, on, off}.
    disp.state().xmm[3].lo    = 0x8000'0000'0000'0000ull;
    disp.state().xmm[3].hi    = 0u;
    disp.state().ymm_hi[3].lo = 0x8000'0000'0000'0000ull;
    disp.state().ymm_hi[3].hi = 0u;
    disp.state().xmm[1].lo    = 0xAAAA'AAAA'BBBB'BBBBull;
    disp.state().xmm[1].hi    = 0xCCCC'CCCC'DDDD'DDDDull;
    disp.state().ymm_hi[1].lo = 0xEEEE'EEEE'FFFF'FFFFull;
    disp.state().ymm_hi[1].hi = 0x1111'1111'2222'2222ull;

    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[1].lo == 1003ull);              // table[3]
    REQUIRE(disp.state().xmm[1].hi == 0xCCCC'CCCC'DDDD'DDDDull);
    REQUIRE(disp.state().ymm_hi[1].lo == 1001ull);           // table[1]
    REQUIRE(disp.state().ymm_hi[1].hi == 0x1111'1111'2222'2222ull);
    REQUIRE(disp.state().xmm[3].lo == 0u);
    REQUIRE(disp.state().ymm_hi[3].lo == 0u);
}

TEST_CASE("e2e: VPGATHERQD xmm1, [rax + ymm2*4], xmm3 — F2-IR-059 ymm index, xmm dest") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }

    translator::Translator tx;
    std::vector<std::uint8_t> bytes{
        0xC4, 0xE2, 0x65, 0x91, 0x0C, 0x90,  // vpgatherqd xmm1,[rax+ymm2*4],xmm3
        0xC3,                                 // ret
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= bytes.size()) return {};
        return std::span<const std::uint8_t>(bytes.data() + off,
                                             bytes.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();

    alignas(16) std::array<std::uint32_t, 8> table{
        100u, 101u, 102u, 103u, 104u, 105u, 106u, 107u};
    disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rax)] =
        reinterpret_cast<std::uint64_t>(table.data());
    // Four qword indices across the ymm: {6, P64, 2, P64}.
    disp.state().xmm[2].lo    = 6ull;
    disp.state().xmm[2].hi    = 0x4000'0000'0000'0000ull;
    disp.state().ymm_hi[2].lo = 2ull;
    disp.state().ymm_hi[2].hi = 0x4000'0000'0000'0000ull;
    // Dword mask lanes 0..3: lanes 0 and 2 active.
    disp.state().xmm[3].lo    = 0x0000'0000'8000'0000ull;
    disp.state().xmm[3].hi    = 0x0000'0000'8000'0000ull;
    disp.state().ymm_hi[3].lo = 0x7777'7777'7777'7777ull;  // junk → 0
    disp.state().xmm[1].lo    = 0xAAAA'AAAA'BBBB'BBBBull;
    disp.state().xmm[1].hi    = 0xCCCC'CCCC'DDDD'DDDDull;
    disp.state().ymm_hi[1].lo = 0x9999'9999'9999'9999ull;  // junk → 0

    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    // Lane 0 ← table[6], lane 1 kept, lane 2 ← table[2], lane 3 kept.
    REQUIRE(disp.state().xmm[1].lo == ((0xAAAA'AAAAull << 32) | 106ull));
    REQUIRE(disp.state().xmm[1].hi == ((0xCCCC'CCCCull << 32) | 102ull));
    // VEX.256 with an xmm dest zeroes bits 255:128, and the mask
    // (including its untouched upper half) reads as zero.
    REQUIRE(disp.state().ymm_hi[1].lo == 0u);
    REQUIRE(disp.state().xmm[3].lo == 0u);
    REQUIRE(disp.state().xmm[3].hi == 0u);
    REQUIRE(disp.state().ymm_hi[3].lo == 0u);
}

// ---- F2-IR-060 SHA-NI e2e: JIT output vs exact SDM references ----
namespace sha_e2e {

using V4 = std::array<std::uint32_t, 4>;  // lane 0 = bits 31:0
using Xmm = std::pair<std::uint64_t, std::uint64_t>;

inline Xmm pack(const V4& v) {
    return {(static_cast<std::uint64_t>(v[1]) << 32) | v[0],
            (static_cast<std::uint64_t>(v[3]) << 32) | v[2]};
}

inline std::uint32_t rol(std::uint32_t x, unsigned n) {
    return (x << n) | (x >> (32u - n));
}
inline std::uint32_t ror(std::uint32_t x, unsigned n) {
    return (x >> n) | (x << (32u - n));
}

// Intel SDM pseudocode, lane-exact.
inline V4 ref_sha1rnds4(const V4& s, const V4& m, unsigned imm) {
    std::uint32_t A = s[3], B = s[2], C = s[1], D = s[0];
    const std::uint32_t W[4] = {m[3], m[2], m[1], m[0]};  // {W0E,W1,W2,W3}
    static constexpr std::uint32_t K[4] = {
        0x5A827999u, 0x6ED9EBA1u, 0x8F1BBCDCu, 0xCA62C1D6u};
    std::uint32_t E = 0;  // round 0's E lives inside W0E
    for (int i = 0; i < 4; ++i) {
        std::uint32_t f = 0;
        switch (imm & 3u) {
            case 0:  f = (B & C) ^ (~B & D);          break;
            case 2:  f = (B & C) ^ (B & D) ^ (C & D); break;
            default: f = B ^ C ^ D;                   break;
        }
        const std::uint32_t t = f + rol(A, 5) + W[i] + E + K[imm & 3u];
        E = D; D = C; C = rol(B, 30); B = A; A = t;
    }
    return {D, C, B, A};
}

inline V4 ref_sha1nexte(const V4& a, const V4& b) {
    V4 r = b;
    r[3] = b[3] + rol(a[3], 30);
    return r;
}

inline V4 ref_sha1msg1(const V4& a, const V4& b) {
    const std::uint32_t W0 = a[3], W1 = a[2], W2 = a[1], W3 = a[0];
    const std::uint32_t W4 = b[3], W5 = b[2];  // b lanes 1,0 ignored
    return {W5 ^ W3, W4 ^ W2, W3 ^ W1, W2 ^ W0};
}

inline V4 ref_sha1msg2(const V4& a, const V4& b) {
    const std::uint32_t W13 = b[2], W14 = b[1], W15 = b[0];  // b lane3 ignored
    const std::uint32_t W16 = rol(a[3] ^ W13, 1);
    const std::uint32_t W17 = rol(a[2] ^ W14, 1);
    const std::uint32_t W18 = rol(a[1] ^ W15, 1);
    const std::uint32_t W19 = rol(a[0] ^ W16, 1);  // chained on W16
    return {W19, W18, W17, W16};
}

inline V4 ref_sha256rnds2(const V4& cdgh, const V4& abef,
                          std::uint32_t wk0, std::uint32_t wk1) {
    std::uint32_t A = abef[3], B = abef[2], E = abef[1], F = abef[0];
    std::uint32_t C = cdgh[3], D = cdgh[2], G = cdgh[1], H = cdgh[0];
    const std::uint32_t WK[2] = {wk0, wk1};
    for (int i = 0; i < 2; ++i) {
        const std::uint32_t s1 = ror(E, 6) ^ ror(E, 11) ^ ror(E, 25);
        const std::uint32_t ch = (E & F) ^ (~E & G);
        const std::uint32_t t1 = H + s1 + ch + WK[i];
        const std::uint32_t s0 = ror(A, 2) ^ ror(A, 13) ^ ror(A, 22);
        const std::uint32_t mj = (A & B) ^ (A & C) ^ (B & C);
        const std::uint32_t t2 = s0 + mj;
        H = G; G = F; F = E; E = D + t1; D = C; C = B; B = A; A = t1 + t2;
    }
    return {F, E, B, A};
}

inline std::uint32_t sig0(std::uint32_t x) {
    return ror(x, 7) ^ ror(x, 18) ^ (x >> 3);
}
inline std::uint32_t sig1(std::uint32_t x) {
    return ror(x, 17) ^ ror(x, 19) ^ (x >> 10);
}

inline V4 ref_sha256msg1(const V4& a, const V4& b) {
    return {a[0] + sig0(a[1]), a[1] + sig0(a[2]),
            a[2] + sig0(a[3]), a[3] + sig0(b[0])};  // b lanes 1..3 ignored
}

inline V4 ref_sha256msg2(const V4& a, const V4& b) {
    const std::uint32_t W14 = b[2], W15 = b[3];  // b lanes 0,1 ignored
    const std::uint32_t W16 = a[0] + sig1(W14);
    const std::uint32_t W17 = a[1] + sig1(W15);
    const std::uint32_t W18 = a[2] + sig1(W16);  // chained
    const std::uint32_t W19 = a[3] + sig1(W17);  // chained
    return {W16, W17, W18, W19};
}

// The VecSha lowering emits ARMv8 crypto instructions unconditionally;
// an ARM64 host without the optional SHA extensions would SIGILL with
// no in-JIT recovery. Execution tests skip there (Codex review
// 2026-06-11). Honour any test override so CPUID tests can't leak
// state into these.
inline bool host_has_sha_crypto() {
    const auto& hf = runtime::host_features();
    return hf.feat_sha1 && hf.feat_sha256;
}

// Same SIGILL caveat for the VecAes lowering (AESE/AESD/AESMC/AESIMC).
inline bool host_has_aes_crypto() {
    return runtime::host_features().feat_aes;
}

struct Result {
    Xmm xmm1;
    Xmm xmm2;
    bool halted;
};

// Runs `body` + RET at guest 0x4000 with xmm0/1/2 seeded.
inline Result run(std::vector<std::uint8_t> body,
                  Xmm x0, Xmm x1, Xmm x2) {
    translator::Translator tx;
    body.push_back(0xC3);  // ret
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= body.size()) return {};
        return std::span<const std::uint8_t>(body.data() + off,
                                             body.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    disp.state().xmm[0].lo = x0.first; disp.state().xmm[0].hi = x0.second;
    disp.state().xmm[1].lo = x1.first; disp.state().xmm[1].hi = x1.second;
    disp.state().xmm[2].lo = x2.first; disp.state().xmm[2].hi = x2.second;
    auto r = disp.run(0x4000, 100);
    return {{disp.state().xmm[1].lo, disp.state().xmm[1].hi},
            {disp.state().xmm[2].lo, disp.state().xmm[2].hi},
            r.exit == runtime::DispatchExit::Halted};
}

}  // namespace sha_e2e

TEST_CASE("e2e: SHA1RNDS4 all selectors vs SDM reference — F2-IR-060") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    if (!sha_e2e::host_has_sha_crypto()) {
        SUCCEED("ARM64 host lacks SHA crypto extensions");
        return;
    }
    using namespace sha_e2e;
    const V4 state = {0x76543210u, 0xFEDCBA98u, 0x89ABCDEFu, 0x01234567u};
    const V4 msg   = {0x80000001u, 0x7FFFFFFFu, 0xDEADBEEFu, 0xC0FFEE00u};
    for (unsigned imm = 0; imm < 4; ++imm) {
        // sha1rnds4 xmm1, xmm2, imm (ModRM CA: reg=xmm1 rm=xmm2).
        auto r = run({0x0F, 0x3A, 0xCC, 0xCA,
                      static_cast<std::uint8_t>(imm)},
                     {0, 0}, pack(state), pack(msg));
        REQUIRE(r.halted);
        REQUIRE(r.xmm1 == pack(ref_sha1rnds4(state, msg, imm)));
    }
    // imm8 upper bits are ignored: 0xE6 behaves as 2.
    auto r = run({0x0F, 0x3A, 0xCC, 0xCA, 0xE6},
                 {0, 0}, pack(state), pack(msg));
    REQUIRE(r.halted);
    REQUIRE(r.xmm1 == pack(ref_sha1rnds4(state, msg, 2u)));
}

TEST_CASE("e2e: SHA1NEXTE vs SDM reference — F2-IR-060") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    if (!sha_e2e::host_has_sha_crypto()) {
        SUCCEED("ARM64 host lacks SHA crypto extensions");
        return;
    }
    using namespace sha_e2e;
    // Asymmetric lane-3 pattern catches ROL30-vs-ROR30; lower lanes
    // of the result must come from the SOURCE, not the dest.
    const V4 a = {0x11111111u, 0x22222222u, 0x33333333u, 0x80000001u};
    const V4 b = {0xA0A0A0A0u, 0xB0B0B0B0u, 0xC0C0C0C0u, 0x80000000u};
    auto r = run({0x0F, 0x38, 0xC8, 0xCA}, {0, 0}, pack(a), pack(b));
    REQUIRE(r.halted);
    REQUIRE(r.xmm1 == pack(ref_sha1nexte(a, b)));
}

TEST_CASE("e2e: SHA1MSG1 ignores src2 low half — F2-IR-060") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    if (!sha_e2e::host_has_sha_crypto()) {
        SUCCEED("ARM64 host lacks SHA crypto extensions");
        return;
    }
    using namespace sha_e2e;
    const V4 a = {0x00000004u, 0x00000003u, 0x00000002u, 0x00000001u};
    const V4 b = {0xDEADDEADu, 0xBEEFBEEFu, 0x00000006u, 0x00000005u};
    auto r = run({0x0F, 0x38, 0xC9, 0xCA}, {0, 0}, pack(a), pack(b));
    REQUIRE(r.halted);
    REQUIRE(r.xmm1 == pack(ref_sha1msg1(a, b)));
}

TEST_CASE("e2e: SHA1MSG2 lane chaining + ROL1 carry — F2-IR-060") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    if (!sha_e2e::host_has_sha_crypto()) {
        SUCCEED("ARM64 host lacks SHA crypto extensions");
        return;
    }
    using namespace sha_e2e;
    // Bit-31-set inputs exercise the rotate-across-carry; b lane 3
    // (W12) is garbage the instruction must ignore.
    const V4 a = {0x80000000u, 0xC0000001u, 0x9999AAAAu, 0xF0F0F0F0u};
    const V4 b = {0x80000003u, 0x7FFFFFFEu, 0x12345678u, 0xDEADC0DEu};
    auto r = run({0x0F, 0x38, 0xCA, 0xCA}, {0, 0}, pack(a), pack(b));
    REQUIRE(r.halted);
    REQUIRE(r.xmm1 == pack(ref_sha1msg2(a, b)));
}

TEST_CASE("e2e: SHA256RNDS2 + ping-pong chaining vs reference — F2-IR-060") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    if (!sha_e2e::host_has_sha_crypto()) {
        SUCCEED("ARM64 host lacks SHA crypto extensions");
        return;
    }
    using namespace sha_e2e;
    // SHA-256("abc") schedule-0 state: H0..H7 split {CDGH}/{ABEF}.
    const V4 cdgh = {0x5BE0CD19u, 0x1F83D9ABu, 0xA54FF53Au, 0x3C6EF372u};
    const V4 abef = {0x9B05688Cu, 0x510E527Fu, 0xBB67AE85u, 0x6A09E667u};
    const std::uint32_t wk0 = 0x428A2F98u + 0x61626380u;
    const std::uint32_t wk1 = 0x71374491u;
    // XMM0 upper lanes are garbage the instruction must ignore.
    const Xmm x0 = {(static_cast<std::uint64_t>(wk1) << 32) | wk0,
                    0xDEADBEEF'CAFEF00Dull};
    // Two chained ops: sha256rnds2 xmm1, xmm2 then xmm2, xmm1 —
    // the architectural ping-pong (new CDGH = old ABEF source).
    auto r = run({0x0F, 0x38, 0xCB, 0xCA,    // xmm1 <- rounds(xmm1, xmm2)
                  0x0F, 0x38, 0xCB, 0xD1},   // xmm2 <- rounds(xmm2, xmm1')
                 x0, pack(cdgh), pack(abef));
    REQUIRE(r.halted);
    const V4 first = ref_sha256rnds2(cdgh, abef, wk0, wk1);
    REQUIRE(r.xmm1 == pack(first));
    REQUIRE(r.xmm2 == pack(ref_sha256rnds2(abef, first, wk0, wk1)));
}

TEST_CASE("e2e: SHA256MSG1 vs SDM reference — F2-IR-060") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    if (!sha_e2e::host_has_sha_crypto()) {
        SUCCEED("ARM64 host lacks SHA crypto extensions");
        return;
    }
    using namespace sha_e2e;
    // MSB-set lanes distinguish SHR3 (logical) from a rotate; b lanes
    // 1..3 are garbage the instruction must ignore.
    const V4 a = {0x80000000u, 0x61626380u, 0x000002A0u, 0xFEDCBA98u};
    const V4 b = {0x80000001u, 0xBAADF00Du, 0xDEADBEEFu, 0xCAFEF00Du};
    auto r = run({0x0F, 0x38, 0xCC, 0xCA}, {0, 0}, pack(a), pack(b));
    REQUIRE(r.halted);
    REQUIRE(r.xmm1 == pack(ref_sha256msg1(a, b)));
}

TEST_CASE("e2e: SHA256MSG2 lane chaining vs SDM reference — F2-IR-060") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    if (!sha_e2e::host_has_sha_crypto()) {
        SUCCEED("ARM64 host lacks SHA crypto extensions");
        return;
    }
    using namespace sha_e2e;
    // Lanes 2,3 depend on the freshly computed lanes 0,1; b lanes
    // 0,1 (W12,W13) are garbage the instruction must ignore.
    const V4 a = {0x00010203u, 0x84858687u, 0x08090A0Bu, 0x8C8D8E8Fu};
    const V4 b = {0xDEADDEADu, 0xBEEFBEEFu, 0x80000400u, 0xC0001000u};
    auto r = run({0x0F, 0x38, 0xCD, 0xCA}, {0, 0}, pack(a), pack(b));
    REQUIRE(r.halted);
    REQUIRE(r.xmm1 == pack(ref_sha256msg2(a, b)));
}

// ---- F2-IR-060 follow-up: full-digest KATs (FIPS 180-4) ------------
//
// The per-instruction tests above pin each SHA-NI op against the SDM
// in isolation; these run the *canonical* SHA-NI compression loops
// (the Intel whitepaper register dance, fully unrolled) over real
// padded messages and compare the final digests against FIPS 180-4
// known answers. That catches cross-instruction composition bugs —
// lane-convention mismatches between ops, WK plumbing, schedule
// chaining — that no single-instruction test can see.
//
// Each KAT also evaluates a host-side mirror of the exact same
// instruction sequence built from the sha_e2e::ref_* SDM models.
// The mirror is asserted on every host (including x86_64 CI), so a
// bug in the test's own orchestration is caught everywhere; the JIT
// half runs under the usual ARM64 guard.
namespace sha_kat {

using sha_e2e::V4;
using Bytes = std::vector<std::uint8_t>;

// x86 encodings, registers 0..7 only (no REX needed). Memory operands
// are always [base + disp32] (mod=10); base must not be rsp/rbp.
inline std::uint8_t modrm_rr(unsigned reg, unsigned rm) {
    return static_cast<std::uint8_t>(0xC0u | (reg << 3) | rm);
}
inline void emit_mem(Bytes& o, unsigned reg, unsigned base,
                     std::uint32_t disp) {
    o.push_back(static_cast<std::uint8_t>(0x80u | (reg << 3) | base));
    o.push_back(static_cast<std::uint8_t>(disp));
    o.push_back(static_cast<std::uint8_t>(disp >> 8));
    o.push_back(static_cast<std::uint8_t>(disp >> 16));
    o.push_back(static_cast<std::uint8_t>(disp >> 24));
}
inline void movdqu_load(Bytes& o, unsigned x, unsigned base,
                        std::uint32_t d) {
    o.insert(o.end(), {0xF3, 0x0F, 0x6F}); emit_mem(o, x, base, d);
}
inline void movdqa_store(Bytes& o, unsigned x, unsigned base,
                         std::uint32_t d) {
    o.insert(o.end(), {0x66, 0x0F, 0x7F}); emit_mem(o, x, base, d);
}
inline void movdqa_rr(Bytes& o, unsigned dst, unsigned src) {
    o.insert(o.end(), {0x66, 0x0F, 0x6F, modrm_rr(dst, src)});
}
inline void paddd_rr(Bytes& o, unsigned dst, unsigned src) {
    o.insert(o.end(), {0x66, 0x0F, 0xFE, modrm_rr(dst, src)});
}
inline void paddd_mem(Bytes& o, unsigned dst, unsigned base,
                      std::uint32_t d) {
    o.insert(o.end(), {0x66, 0x0F, 0xFE}); emit_mem(o, dst, base, d);
}
inline void pshufd_rr(Bytes& o, unsigned dst, unsigned src,
                      std::uint8_t imm) {
    o.insert(o.end(), {0x66, 0x0F, 0x70, modrm_rr(dst, src), imm});
}
inline void palignr_rr(Bytes& o, unsigned dst, unsigned src,
                       std::uint8_t imm) {
    o.insert(o.end(), {0x66, 0x0F, 0x3A, 0x0F, modrm_rr(dst, src), imm});
}
inline void pxor_rr(Bytes& o, unsigned dst, unsigned src) {
    o.insert(o.end(), {0x66, 0x0F, 0xEF, modrm_rr(dst, src)});
}
inline void sha256rnds2(Bytes& o, unsigned dst, unsigned src) {
    o.insert(o.end(), {0x0F, 0x38, 0xCB, modrm_rr(dst, src)});
}
inline void sha256msg1(Bytes& o, unsigned dst, unsigned src) {
    o.insert(o.end(), {0x0F, 0x38, 0xCC, modrm_rr(dst, src)});
}
inline void sha256msg2(Bytes& o, unsigned dst, unsigned src) {
    o.insert(o.end(), {0x0F, 0x38, 0xCD, modrm_rr(dst, src)});
}
inline void sha1rnds4(Bytes& o, unsigned dst, unsigned src,
                      std::uint8_t imm) {
    o.insert(o.end(), {0x0F, 0x3A, 0xCC, modrm_rr(dst, src), imm});
}
inline void sha1nexte_rr(Bytes& o, unsigned dst, unsigned src) {
    o.insert(o.end(), {0x0F, 0x38, 0xC8, modrm_rr(dst, src)});
}
inline void sha1nexte_mem(Bytes& o, unsigned dst, unsigned base,
                          std::uint32_t d) {
    o.insert(o.end(), {0x0F, 0x38, 0xC8}); emit_mem(o, dst, base, d);
}
inline void sha1msg1(Bytes& o, unsigned dst, unsigned src) {
    o.insert(o.end(), {0x0F, 0x38, 0xC9, modrm_rr(dst, src)});
}
inline void sha1msg2(Bytes& o, unsigned dst, unsigned src) {
    o.insert(o.end(), {0x0F, 0x38, 0xCA, modrm_rr(dst, src)});
}

constexpr unsigned kRcx = 1;  // -> K constant table (SHA-256 only)
constexpr unsigned kRdx = 2;  // -> schedule words, 16 dwords per block
constexpr unsigned kRbx = 3;  // -> 32-byte per-block state-save scratch

// FIPS 180-4 §5.1.1 padding (shared by SHA-1 / SHA-256): append 0x80,
// zero-fill to 56 mod 64, then the 64-bit big-endian bit length.
// Returns the schedule words W (big-endian dwords re-parsed to native),
// 16 per block.
inline std::vector<std::uint32_t> pad_to_words(const char* msg,
                                               std::size_t len) {
    std::vector<std::uint8_t> bytes(msg, msg + len);
    const std::uint64_t bit_len = static_cast<std::uint64_t>(len) * 8u;
    bytes.push_back(0x80u);
    while (bytes.size() % 64u != 56u) bytes.push_back(0x00u);
    for (int i = 7; i >= 0; --i) {
        bytes.push_back(static_cast<std::uint8_t>(bit_len >> (8 * i)));
    }
    std::vector<std::uint32_t> w(bytes.size() / 4u);
    for (std::size_t i = 0; i < w.size(); ++i) {
        w[i] = (std::uint32_t{bytes[4 * i]} << 24) |
               (std::uint32_t{bytes[4 * i + 1]} << 16) |
               (std::uint32_t{bytes[4 * i + 2]} << 8) |
               std::uint32_t{bytes[4 * i + 3]};
    }
    return w;
}

inline V4 lane_add(const V4& a, const V4& b) {
    return {a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]};
}
inline V4 lane_xor(const V4& a, const V4& b) {
    return {a[0] ^ b[0], a[1] ^ b[1], a[2] ^ b[2], a[3] ^ b[3]};
}
// PALIGNR dst, src, 4 == lanes {src[1], src[2], src[3], dst[0]}.
inline V4 alignr4(const V4& dst, const V4& src) {
    return {src[1], src[2], src[3], dst[0]};
}
inline V4 unpack(std::uint64_t lo, std::uint64_t hi) {
    return {static_cast<std::uint32_t>(lo),
            static_cast<std::uint32_t>(lo >> 32),
            static_cast<std::uint32_t>(hi),
            static_cast<std::uint32_t>(hi >> 32)};
}

alignas(16) constexpr std::uint32_t kSha256K[64] = {
    0x428A2F98u, 0x71374491u, 0xB5C0FBCFu, 0xE9B5DBA5u, 0x3956C25Bu,
    0x59F111F1u, 0x923F82A4u, 0xAB1C5ED5u, 0xD807AA98u, 0x12835B01u,
    0x243185BEu, 0x550C7DC3u, 0x72BE5D74u, 0x80DEB1FEu, 0x9BDC06A7u,
    0xC19BF174u, 0xE49B69C1u, 0xEFBE4786u, 0x0FC19DC6u, 0x240CA1CCu,
    0x2DE92C6Fu, 0x4A7484AAu, 0x5CB0A9DCu, 0x76F988DAu, 0x983E5152u,
    0xA831C66Du, 0xB00327C8u, 0xBF597FC7u, 0xC6E00BF3u, 0xD5A79147u,
    0x06CA6351u, 0x14292967u, 0x27B70A85u, 0x2E1B2138u, 0x4D2C6DFCu,
    0x53380D13u, 0x650A7354u, 0x766A0ABBu, 0x81C2C92Eu, 0x92722C85u,
    0xA2BFE8A1u, 0xA81A664Bu, 0xC24B8B70u, 0xC76C51A3u, 0xD192E819u,
    0xD6990624u, 0xF40E3585u, 0x106AA070u, 0x19A4C116u, 0x1E376C08u,
    0x2748774Cu, 0x34B0BCB5u, 0x391C0CB3u, 0x4ED8AA4Au, 0x5B9CCA4Fu,
    0x682E6FF3u, 0x748F82EEu, 0x78A5636Fu, 0x84C87814u, 0x8CC70208u,
    0x90BEFFFAu, 0xA4506CEBu, 0xBEF9A3F7u, 0xC67178F2u};

// Canonical unrolled SHA-NI SHA-256 compression for `blocks` blocks.
// Register convention (Intel whitepaper): xmm1 = {ABEF} (A in lane 3),
// xmm2 = {CDGH}, xmm0 = WK, xmm3-6 = schedule, xmm7 = palignr temp.
// `jmp +0` between blocks keeps each translated block modest.
inline Bytes gen_sha256(unsigned blocks) {
    Bytes o;
    const unsigned M[4] = {3, 4, 5, 6};
    for (unsigned b = 0; b < blocks; ++b) {
        if (b != 0) o.insert(o.end(), {0xEB, 0x00});
        movdqa_store(o, 1, kRbx, 0);
        movdqa_store(o, 2, kRbx, 16);
        for (unsigned j = 0; j < 4; ++j) {
            movdqu_load(o, M[j], kRdx, 64u * b + 16u * j);
        }
        for (unsigned g = 0; g < 16; ++g) {
            movdqa_rr(o, 0, M[g % 4]);
            paddd_mem(o, 0, kRcx, 16u * g);   // xmm0 = W + K
            sha256rnds2(o, 2, 1);             // rounds 4g, 4g+1
            pshufd_rr(o, 0, 0, 0x0E);         // WK2/WK3 into lanes 0,1
            sha256rnds2(o, 1, 2);             // rounds 4g+2, 4g+3
            if (g < 12) {                     // W[4(g+4)..4(g+4)+3]
                movdqa_rr(o, 7, M[(g + 3) % 4]);
                palignr_rr(o, 7, M[(g + 2) % 4], 4);
                sha256msg1(o, M[g % 4], M[(g + 1) % 4]);
                paddd_rr(o, M[g % 4], 7);
                sha256msg2(o, M[g % 4], M[(g + 3) % 4]);
            }
        }
        paddd_mem(o, 1, kRbx, 0);
        paddd_mem(o, 2, kRbx, 16);
    }
    return o;
}

// Host-side mirror of gen_sha256 built on the sha_e2e::ref_* models —
// same register dance, same lane conventions, no JIT.
inline void host_sha256(const std::uint32_t* w, unsigned blocks,
                        V4& abef, V4& cdgh) {
    using sha_e2e::ref_sha256rnds2;
    using sha_e2e::ref_sha256msg1;
    using sha_e2e::ref_sha256msg2;
    for (unsigned b = 0; b < blocks; ++b) {
        const V4 abef0 = abef, cdgh0 = cdgh;
        V4 m[4];
        for (unsigned j = 0; j < 4; ++j) {
            const std::uint32_t* p = w + 16u * b + 4u * j;
            m[j] = {p[0], p[1], p[2], p[3]};
        }
        for (unsigned g = 0; g < 16; ++g) {
            const V4 k = {kSha256K[4 * g], kSha256K[4 * g + 1],
                          kSha256K[4 * g + 2], kSha256K[4 * g + 3]};
            const V4 wk = lane_add(m[g % 4], k);
            cdgh = ref_sha256rnds2(cdgh, abef, wk[0], wk[1]);
            abef = ref_sha256rnds2(abef, cdgh, wk[2], wk[3]);
            if (g < 12) {
                const V4 t = alignr4(m[(g + 3) % 4], m[(g + 2) % 4]);
                m[g % 4] = ref_sha256msg2(
                    lane_add(ref_sha256msg1(m[g % 4], m[(g + 1) % 4]), t),
                    m[(g + 3) % 4]);
            }
        }
        abef = lane_add(abef, abef0);
        cdgh = lane_add(cdgh, cdgh0);
    }
}

// Canonical unrolled SHA-NI SHA-1 compression. xmm1 = {ABCD} (A in
// lane 3); xmm2/xmm3 ping-pong the E+W operand (the sha1nexte chain);
// xmm4-7 = schedule with W0-in-lane-3 group order. K is implicit in
// SHA1RNDS4's immediate, so there is no constant table.
inline Bytes gen_sha1(unsigned blocks) {
    Bytes o;
    const unsigned M[4] = {4, 5, 6, 7};
    for (unsigned b = 0; b < blocks; ++b) {
        if (b != 0) o.insert(o.end(), {0xEB, 0x00});
        movdqa_store(o, 1, kRbx, 0);   // {ABCD} save
        movdqa_store(o, 2, kRbx, 16);  // {0,0,0,E} save
        for (unsigned j = 0; j < 4; ++j) {
            movdqu_load(o, M[j], kRdx, 64u * b + 16u * j);
        }
        for (unsigned g = 0; g < 20; ++g) {
            const unsigned e_cur = (g % 2 == 0) ? 2 : 3;
            const unsigned e_nxt = (g % 2 == 0) ? 3 : 2;
            if (g == 0) {
                paddd_rr(o, e_cur, M[0]);  // {0,0,0,E} + {W0..W3}
            } else {
                sha1nexte_rr(o, e_cur, M[g % 4]);
            }
            movdqa_rr(o, e_nxt, 1);        // ABCD snapshot for group g+1
            sha1rnds4(o, 1, e_cur, static_cast<std::uint8_t>(g / 5));
            if (g < 16) {                  // W[4(g+4)..4(g+4)+3]
                sha1msg1(o, M[g % 4], M[(g + 1) % 4]);
                pxor_rr(o, M[g % 4], M[(g + 2) % 4]);
                sha1msg2(o, M[g % 4], M[(g + 3) % 4]);
            }
        }
        // Group 19's snapshot (pre-final-rnds4 ABCD) sits in xmm2;
        // E' = saved E + rol30(its lane 3) via nexte against the save.
        sha1nexte_mem(o, 2, kRbx, 16);
        paddd_mem(o, 1, kRbx, 0);
    }
    return o;
}

// Host-side mirror of gen_sha1 on the sha_e2e::ref_* models.
inline void host_sha1(const std::uint32_t* w, unsigned blocks,
                      V4& abcd, V4& e_vec) {
    using sha_e2e::ref_sha1rnds4;
    using sha_e2e::ref_sha1nexte;
    using sha_e2e::ref_sha1msg1;
    using sha_e2e::ref_sha1msg2;
    for (unsigned b = 0; b < blocks; ++b) {
        const V4 abcd0 = abcd, e0_save = e_vec;
        V4 m[4];
        for (unsigned j = 0; j < 4; ++j) {
            const std::uint32_t* p = w + 16u * b + 4u * j;
            m[j] = {p[3], p[2], p[1], p[0]};  // W0 in lane 3
        }
        V4 e[2] = {e_vec, {}};
        for (unsigned g = 0; g < 20; ++g) {
            const unsigned cur = g % 2, nxt = 1 - cur;
            if (g == 0) {
                e[cur] = lane_add(e[cur], m[0]);
            } else {
                e[cur] = ref_sha1nexte(e[cur], m[g % 4]);
            }
            e[nxt] = abcd;
            abcd = ref_sha1rnds4(abcd, e[cur], g / 5);
            if (g < 16) {
                m[g % 4] = ref_sha1msg2(
                    lane_xor(ref_sha1msg1(m[g % 4], m[(g + 1) % 4]),
                             m[(g + 2) % 4]),
                    m[(g + 3) % 4]);
            }
        }
        e_vec = ref_sha1nexte(e[0], e0_save);
        abcd = lane_add(abcd, abcd0);
    }
}

struct JitState {
    V4 xmm1;
    V4 xmm2;
    bool halted;
};

// Runs the generated program + RET with rcx/rdx/rbx and xmm1/xmm2
// seeded, returning the final state registers.
inline JitState run_kat(Bytes body, std::uint64_t k_ptr,
                        std::uint64_t msg_ptr, std::uint64_t scratch_ptr,
                        sha_e2e::Xmm x1, sha_e2e::Xmm x2) {
    translator::Translator tx;
    body.push_back(0xC3);  // ret
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= body.size()) return {};
        return std::span<const std::uint8_t>(body.data() + off,
                                             body.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    disp.state()[ir::Gpr::Rcx] = k_ptr;
    disp.state()[ir::Gpr::Rdx] = msg_ptr;
    disp.state()[ir::Gpr::Rbx] = scratch_ptr;
    disp.state().xmm[1].lo = x1.first; disp.state().xmm[1].hi = x1.second;
    disp.state().xmm[2].lo = x2.first; disp.state().xmm[2].hi = x2.second;
    auto r = disp.run(0x4000, 16);
    return {unpack(disp.state().xmm[1].lo, disp.state().xmm[1].hi),
            unpack(disp.state().xmm[2].lo, disp.state().xmm[2].hi),
            r.exit == runtime::DispatchExit::Halted};
}

struct Kat {
    const char* msg;
    std::size_t len;
};

// FIPS 180-4 / NIST CAVP known answers: empty, "abc" (one block),
// and the 448-bit two-block message.
constexpr const char* kTwoBlockMsg =
    "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";

constexpr Kat kKats[3] = {{"", 0}, {"abc", 3}, {kTwoBlockMsg, 56}};

constexpr std::uint32_t kSha256Expected[3][8] = {
    {0xE3B0C442u, 0x98FC1C14u, 0x9AFBF4C8u, 0x996FB924u,
     0x27AE41E4u, 0x649B934Cu, 0xA495991Bu, 0x7852B855u},
    {0xBA7816BFu, 0x8F01CFEAu, 0x414140DEu, 0x5DAE2223u,
     0xB00361A3u, 0x96177A9Cu, 0xB410FF61u, 0xF20015ADu},
    {0x248D6A61u, 0xD20638B8u, 0xE5C02693u, 0x0C3E6039u,
     0xA33CE459u, 0x64FF2167u, 0xF6ECEDD4u, 0x19DB06C1u}};

constexpr std::uint32_t kSha1Expected[3][5] = {
    {0xDA39A3EEu, 0x5E6B4B0Du, 0x3255BFEFu, 0x95601890u, 0xAFD80709u},
    {0xA9993E36u, 0x4706816Au, 0xBA3E2571u, 0x7850C26Cu, 0x9CD0D89Du},
    {0x84983E44u, 0x1C3BD26Eu, 0xBAAE4AA1u, 0xF95129E5u, 0xE54670F1u}};

}  // namespace sha_kat

TEST_CASE("e2e: SHA-256 full-digest KAT via canonical SHA-NI loop "
          "— F2-IR-060") {
    using namespace sha_kat;
    const V4 iv_abef = {0x9B05688Cu, 0x510E527Fu, 0xBB67AE85u, 0x6A09E667u};
    const V4 iv_cdgh = {0x5BE0CD19u, 0x1F83D9ABu, 0xA54FF53Au, 0x3C6EF372u};
    for (std::size_t k = 0; k < 3; ++k) {
        INFO("message #" << k << " (" << kKats[k].len << " bytes)");
        const auto w = pad_to_words(kKats[k].msg, kKats[k].len);
        const unsigned blocks = static_cast<unsigned>(w.size() / 16u);

        // Host-side mirror first: validates the loop orchestration on
        // every host, ARM64 or not.
        V4 abef = iv_abef, cdgh = iv_cdgh;
        host_sha256(w.data(), blocks, abef, cdgh);
        const std::uint32_t host_digest[8] = {
            abef[3], abef[2], cdgh[3], cdgh[2],
            abef[1], abef[0], cdgh[1], cdgh[0]};
        for (int i = 0; i < 8; ++i) {
            REQUIRE(host_digest[i] == kSha256Expected[k][i]);
        }

        if constexpr (!is_arm64) {
            // Execution needs the host arch, but decode + lowering are
            // host-independent: the program must at least translate
            // (catches decoder gaps and scratch-pool exhaustion on
            // x86_64 CI too).
            Bytes body = gen_sha256(blocks);
            body.push_back(0xC3);
            translator::Translator tx;
            auto tr = tx.translate(0x4000, body);
            REQUIRE(std::holds_alternative<translator::TranslatedBlock>(tr));
            continue;
        }
        if (!sha_e2e::host_has_sha_crypto()) {
            SUCCEED("ARM64 host lacks SHA crypto — JIT half skipped");
            break;
        }

        // JIT half: schedule words and K live in host memory the
        // guest addresses through rdx / rcx; rbx points at the
        // per-block state-save scratch.
        alignas(16) std::uint32_t scratch[8] = {};
        const auto body = gen_sha256(blocks);
        auto r = run_kat(body,
                         reinterpret_cast<std::uint64_t>(kSha256K),
                         reinterpret_cast<std::uint64_t>(w.data()),
                         reinterpret_cast<std::uint64_t>(scratch),
                         sha_e2e::pack(iv_abef), sha_e2e::pack(iv_cdgh));
        REQUIRE(r.halted);
        const std::uint32_t jit_digest[8] = {
            r.xmm1[3], r.xmm1[2], r.xmm2[3], r.xmm2[2],
            r.xmm1[1], r.xmm1[0], r.xmm2[1], r.xmm2[0]};
        for (int i = 0; i < 8; ++i) {
            REQUIRE(jit_digest[i] == kSha256Expected[k][i]);
        }
    }
}

TEST_CASE("e2e: SHA-1 full-digest KAT via canonical SHA-NI loop "
          "— F2-IR-060") {
    using namespace sha_kat;
    const V4 iv_abcd = {0x10325476u, 0x98BADCFEu, 0xEFCDAB89u, 0x67452301u};
    const V4 iv_e = {0u, 0u, 0u, 0xC3D2E1F0u};
    for (std::size_t k = 0; k < 3; ++k) {
        INFO("message #" << k << " (" << kKats[k].len << " bytes)");
        const auto w = pad_to_words(kKats[k].msg, kKats[k].len);
        const unsigned blocks = static_cast<unsigned>(w.size() / 16u);

        V4 abcd = iv_abcd, e_vec = iv_e;
        host_sha1(w.data(), blocks, abcd, e_vec);
        const std::uint32_t host_digest[5] = {
            abcd[3], abcd[2], abcd[1], abcd[0], e_vec[3]};
        for (int i = 0; i < 5; ++i) {
            REQUIRE(host_digest[i] == kSha1Expected[k][i]);
        }

        if constexpr (!is_arm64) {
            Bytes body = gen_sha1(blocks);
            body.push_back(0xC3);
            translator::Translator tx;
            auto tr = tx.translate(0x4000, body);
            REQUIRE(std::holds_alternative<translator::TranslatedBlock>(tr));
            continue;
        }
        if (!sha_e2e::host_has_sha_crypto()) {
            SUCCEED("ARM64 host lacks SHA crypto — JIT half skipped");
            break;
        }

        // SHA-1 message registers carry W0 in lane 3, so the guest
        // buffer stores each 4-word group reversed.
        std::vector<std::uint32_t> guest_w(w.size());
        for (std::size_t g = 0; g < w.size() / 4u; ++g) {
            for (std::size_t j = 0; j < 4u; ++j) {
                guest_w[4 * g + j] = w[4 * g + (3 - j)];
            }
        }
        alignas(16) std::uint32_t scratch[8] = {};
        const auto body = gen_sha1(blocks);
        auto r = run_kat(body, /*k_ptr=*/0,
                         reinterpret_cast<std::uint64_t>(guest_w.data()),
                         reinterpret_cast<std::uint64_t>(scratch),
                         sha_e2e::pack(iv_abcd), sha_e2e::pack(iv_e));
        REQUIRE(r.halted);
        const std::uint32_t jit_digest[5] = {
            r.xmm1[3], r.xmm1[2], r.xmm1[1], r.xmm1[0], r.xmm2[3]};
        for (int i = 0; i < 5; ++i) {
            REQUIRE(jit_digest[i] == kSha1Expected[k][i]);
        }
    }
}

namespace {

// RAII so a failing REQUIRE can't leak a host-features override into
// later tests in the same process.
struct FeatureOverrideGuard {
    explicit FeatureOverrideGuard(runtime::HostFeatures f) {
        runtime::override_host_features_for_test(f);
    }
    ~FeatureOverrideGuard() { runtime::clear_host_features_override(); }
};

// Runs `cpuid; ret` with the given RAX/RCX and returns the final
// frame. The Translator is constructed after the override is set —
// CPUID values are baked at translation time.
runtime::CpuStateFrame run_cpuid(std::uint64_t rax, std::uint64_t rcx) {
    translator::Translator tx;
    static const std::vector<std::uint8_t> body{0x0F, 0xA2, 0xC3};
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= body.size()) return {};
        return std::span<const std::uint8_t>(body.data() + off,
                                             body.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    disp.state()[ir::Gpr::Rax] = rax;
    disp.state()[ir::Gpr::Rcx] = rcx;
    auto r = disp.run(0x4000, 4);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    return disp.state();
}

}  // namespace

TEST_CASE("e2e: CPUID leaf model + SHA advertisement — F2-IR-060 followup") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }

    runtime::HostFeatures sha_host{};
    sha_host.feat_sha1 = true;
    sha_host.feat_sha256 = true;

    SECTION("host with SHA crypto advertises CPUID.7.0:EBX bit 29") {
        FeatureOverrideGuard guard{sha_host};
        auto s = run_cpuid(7, 0);
        REQUIRE((s[ir::Gpr::Rbx] & (1ull << 29)) != 0u);
        REQUIRE(s[ir::Gpr::Rax] == 0u);  // max subleaf
        REQUIRE(s[ir::Gpr::Rcx] == 0u);
        REQUIRE(s[ir::Gpr::Rdx] == 0u);
    }
    SECTION("host without SHA crypto keeps the bit clear") {
        FeatureOverrideGuard guard{runtime::HostFeatures{}};
        auto s = run_cpuid(7, 0);
        REQUIRE((s[ir::Gpr::Rbx] & (1ull << 29)) == 0u);
    }
    SECTION("leaf 0 reports max basic leaf 7 + GenuineIntel vendor") {
        FeatureOverrideGuard guard{runtime::HostFeatures{}};
        // Subleaf is ignored for leaf 0.
        auto s = run_cpuid(0, 0xDEADu);
        REQUIRE(s[ir::Gpr::Rax] == 7u);
        REQUIRE(s[ir::Gpr::Rbx] == 0x756E6547u);  // "Genu"
        REQUIRE(s[ir::Gpr::Rdx] == 0x49656E69u);  // "ineI"
        REQUIRE(s[ir::Gpr::Rcx] == 0x6C65746Eu);  // "ntel"
    }
    SECTION("leaf 1 reports signature + honest feature bits") {
        FeatureOverrideGuard guard{runtime::HostFeatures{}};
        auto s = run_cpuid(1, 0);
        REQUIRE(s[ir::Gpr::Rax] == 0x000206A7u);
        // EDX: FPU, TSC, CX8, CMOV, SSE, SSE2 — nothing else.
        REQUIRE(s[ir::Gpr::Rdx] ==
                ((1u << 0) | (1u << 4) | (1u << 8) | (1u << 15) |
                 (1u << 25) | (1u << 26)));
        const std::uint64_t ecx = s[ir::Gpr::Rcx];
        REQUIRE((ecx & (1u << 0)) != 0u);    // SSE3
        REQUIRE((ecx & (1u << 9)) != 0u);    // SSSE3
        REQUIRE((ecx & (1u << 12)) != 0u);   // FMA
        REQUIRE((ecx & (1u << 13)) != 0u);   // CMPXCHG16B
        REQUIRE((ecx & (1u << 19)) != 0u);   // SSE4.1
        REQUIRE((ecx & (1u << 22)) != 0u);   // MOVBE
        REQUIRE((ecx & (1u << 23)) != 0u);   // POPCNT (32/64 + mem)
        REQUIRE((ecx & (1u << 27)) != 0u);   // OSXSAVE
        REQUIRE((ecx & (1u << 28)) != 0u);   // AVX
        REQUIRE((ecx & (1u << 20)) == 0u);   // SSE4.2 off: PCMPxSTRx
        REQUIRE((ecx & (1u << 25)) == 0u);   // AESNI off: no host AES
        REQUIRE((ecx & (1u << 26)) == 0u);   // XSAVE deliberately off
        REQUIRE((ecx & (1u << 1)) == 0u);    // PCLMULQDQ not decoded
    }
    SECTION("AESNI bit follows host AES crypto") {
        runtime::HostFeatures aes_host{};
        aes_host.feat_aes = true;
        FeatureOverrideGuard guard{aes_host};
        auto s = run_cpuid(1, 0);
        REQUIRE((s[ir::Gpr::Rcx] & (1u << 25)) != 0u);
    }
    SECTION("leaf 7 EBX always carries BMI2") {
        FeatureOverrideGuard guard{runtime::HostFeatures{}};
        auto s = run_cpuid(7, 0);
        REQUIRE((s[ir::Gpr::Rbx] & (1u << 8)) != 0u);   // BMI2
        REQUIRE((s[ir::Gpr::Rbx] & (1u << 3)) == 0u);   // BMI1 off
        REQUIRE((s[ir::Gpr::Rbx] & (1u << 5)) == 0u);   // AVX2 off
    }
    SECTION("unmodelled leaves and subleaves return zeros") {
        FeatureOverrideGuard guard{sha_host};
        auto s1 = run_cpuid(2, 0);
        REQUIRE(s1[ir::Gpr::Rax] == 0u);
        REQUIRE(s1[ir::Gpr::Rbx] == 0u);
        auto s2 = run_cpuid(7, 1);  // leaf 7 subleaf 1
        REQUIRE(s2[ir::Gpr::Rbx] == 0u);
    }
    SECTION("basic leaves above the max clamp to leaf 7 per the SDM") {
        FeatureOverrideGuard guard{sha_host};
        auto s1 = run_cpuid(8, 0);
        REQUIRE((s1[ir::Gpr::Rbx] & (1ull << 29)) != 0u);
        auto s2 = run_cpuid(0x12345u, 0);
        REQUIRE((s2[ir::Gpr::Rbx] & (1ull << 29)) != 0u);
        // ... but the clamped view keeps subleaf semantics ...
        auto s3 = run_cpuid(8, 1);
        REQUIRE(s3[ir::Gpr::Rbx] == 0u);
        // ... and the extended range (bit 31) is not clamped into it.
        auto s4 = run_cpuid(0x80000000u, 0);
        REQUIRE(s4[ir::Gpr::Rax] == 0u);
        REQUIRE(s4[ir::Gpr::Rbx] == 0u);
    }
    SECTION("leaf compare reads EAX, not RAX") {
        FeatureOverrideGuard guard{sha_host};
        // Garbage in the upper half of RAX must be ignored, exactly
        // like hardware CPUID.
        auto s = run_cpuid(0xFFFFFFFF'00000007ull, 0);
        REQUIRE((s[ir::Gpr::Rbx] & (1ull << 29)) != 0u);
    }
}

TEST_CASE("e2e: XGETBV reports XCR0 = x87|SSE|AVX — guest AVX gate") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    static const std::vector<std::uint8_t> body{0x0F, 0x01, 0xD0, 0xC3};
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= body.size()) return {};
        return std::span<const std::uint8_t>(body.data() + off,
                                             body.size() - off);
    };

    SECTION("ECX=0 returns the baked XCR0 in EDX:EAX") {
        runtime::Dispatcher disp{tx, reader};
        disp.install_halt_return_stack();
        disp.state()[ir::Gpr::Rcx] = 0;
        disp.state()[ir::Gpr::Rax] = 0xDEADBEEFull;   // overwritten
        disp.state()[ir::Gpr::Rbx] = 0x1234'5678ull;  // must survive
        disp.state()[ir::Gpr::Rdx] = 0xFFFFull;       // overwritten
        auto r = disp.run(0x4000, 4);
        REQUIRE(r.exit == runtime::DispatchExit::Halted);
        REQUIRE(disp.state()[ir::Gpr::Rax] == 0x7u);
        REQUIRE(disp.state()[ir::Gpr::Rdx] == 0u);
        REQUIRE(disp.state()[ir::Gpr::Rbx] == 0x1234'5678ull);
        REQUIRE(disp.state()[ir::Gpr::Rcx] == 0u);  // input preserved
    }
    SECTION("ECX!=0 returns zeros (placeholder for #GP)") {
        runtime::Dispatcher disp{tx, reader};
        disp.install_halt_return_stack();
        disp.state()[ir::Gpr::Rcx] = 1;
        disp.state()[ir::Gpr::Rax] = 0xDEADBEEFull;
        auto r = disp.run(0x4000, 4);
        REQUIRE(r.exit == runtime::DispatchExit::Halted);
        REQUIRE(disp.state()[ir::Gpr::Rax] == 0u);
        REQUIRE(disp.state()[ir::Gpr::Rdx] == 0u);
    }
}

TEST_CASE("e2e: RDTSC is monotonic and non-zero — decoder gap sweep") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    // rdtsc; mov rbx, rax; rdtsc; ret — second reading in EDX:EAX,
    // first stashed in RBX (low 32 bits suffice for the comparison
    // since CNTVCT won't wrap during a test).
    static const std::vector<std::uint8_t> body{
        0x0F, 0x31,
        0x48, 0x89, 0xC3,  // mov rbx, rax
        0x0F, 0x31,
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= body.size()) return {};
        return std::span<const std::uint8_t>(body.data() + off,
                                             body.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto r = disp.run(0x4000, 4);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    const std::uint64_t first = disp.state()[ir::Gpr::Rbx];
    const std::uint64_t second =
        disp.state()[ir::Gpr::Rax] |
        (disp.state()[ir::Gpr::Rdx] << 32);
    REQUIRE(second != 0u);
    REQUIRE(second >= first);
}

TEST_CASE("e2e: CMPXCHG8B success and failure paths + ZF direction") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    auto run_c8b = [](std::uint64_t mem_init, std::uint32_t eax,
                      std::uint32_t edx) {
        translator::Translator tx;
        // lock cmpxchg8b [rsi]; jz +3 (skip mov rdi, 1); ret
        // RDI = 1 on the ZF=0 (failure) path, 2 on success.
        static thread_local std::uint64_t mem;
        mem = mem_init;
        std::vector<std::uint8_t> body{
            0xF0, 0x0F, 0xC7, 0x0E,                          // lock cmpxchg8b [rsi]
            0x74, 0x0B,                                       // jz +11 (to movabs rdi,2)
            0x48, 0xBF, 0x01, 0, 0, 0, 0, 0, 0, 0,            // movabs rdi, 1
            0xC3,                                             // ret
            0x48, 0xBF, 0x02, 0, 0, 0, 0, 0, 0, 0,            // movabs rdi, 2
            0xC3,                                             // ret
        };
        auto reader =
            [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
            if (pc < 0x4000ull) return {};
            const std::size_t off =
                static_cast<std::size_t>(pc - 0x4000ull);
            if (off >= body.size()) return {};
            return std::span<const std::uint8_t>(body.data() + off,
                                                 body.size() - off);
        };
        runtime::Dispatcher disp{tx, reader};
        disp.install_halt_return_stack();
        disp.state()[ir::Gpr::Rsi] = reinterpret_cast<std::uint64_t>(&mem);
        // Garbage above bit 31 only: the compare must read EDX:EAX.
        disp.state()[ir::Gpr::Rax] = (0xAAAA0000ull << 32) | eax;
        disp.state()[ir::Gpr::Rdx] = (0xBBBB0000ull << 32) | edx;
        disp.state()[ir::Gpr::Rbx] = 0x11111111u;
        disp.state()[ir::Gpr::Rcx] = 0x22222222u;
        auto r = disp.run(0x4000, 8);
        REQUIRE(r.exit == runtime::DispatchExit::Halted);
        struct Out {
            std::uint64_t mem, rdi, rax, rdx;
        };
        return Out{mem, disp.state()[ir::Gpr::Rdi],
                   disp.state()[ir::Gpr::Rax],
                   disp.state()[ir::Gpr::Rdx]};
    };

    SECTION("match stores ECX:EBX and sets ZF") {
        auto o = run_c8b(0x00000004'00000003ull, 0x3u, 0x4u);
        REQUIRE(o.mem == 0x22222222'11111111ull);
        REQUIRE(o.rdi == 2u);  // jz taken
    }
    SECTION("mismatch loads m64 into EDX:EAX, clears ZF") {
        auto o = run_c8b(0x00000009'00000008ull, 0x3u, 0x4u);
        REQUIRE(o.mem == 0x00000009'00000008ull);  // unchanged value
        REQUIRE(o.rdi == 1u);  // jz not taken
        REQUIRE(o.rax == 0x8u);  // EAX <- m64 lo (zero-extended)
        REQUIRE(o.rdx == 0x9u);  // EDX <- m64 hi
    }
}

TEST_CASE("e2e: CMPXCHG with rm aliasing RAX takes the dst write") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    // cmpxchg rax, rcx (48 0F B1 C8): rm = rax → compare always
    // succeeds and RAX must end up = RCX (Codex review finding: the
    // unconditional accumulator writeback used to win).
    static const std::vector<std::uint8_t> body{0x48, 0x0F, 0xB1, 0xC8,
                                                0xC3};
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= body.size()) return {};
        return std::span<const std::uint8_t>(body.data() + off,
                                             body.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    disp.state()[ir::Gpr::Rax] = 0x1234u;
    disp.state()[ir::Gpr::Rcx] = 0x5678u;
    auto r = disp.run(0x4000, 4);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state()[ir::Gpr::Rax] == 0x5678u);
}

TEST_CASE("e2e: BSF/BSR real results + src==0 preserves dst") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    auto run_scan = [](std::vector<std::uint8_t> body, std::uint64_t rax,
                       std::uint64_t rcx) {
        translator::Translator tx;
        body.push_back(0xC3);
        auto reader =
            [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
            if (pc < 0x4000ull) return {};
            const std::size_t off =
                static_cast<std::size_t>(pc - 0x4000ull);
            if (off >= body.size()) return {};
            return std::span<const std::uint8_t>(body.data() + off,
                                                 body.size() - off);
        };
        runtime::Dispatcher disp{tx, reader};
        disp.install_halt_return_stack();
        disp.state()[ir::Gpr::Rax] = rax;
        disp.state()[ir::Gpr::Rcx] = rcx;
        auto r = disp.run(0x4000, 4);
        REQUIRE(r.exit == runtime::DispatchExit::Halted);
        return disp.state()[ir::Gpr::Rax];
    };

    // bsf rax, rcx (48 0F BC C1)
    REQUIRE(run_scan({0x48, 0x0F, 0xBC, 0xC1}, 0xDEAD, 0x10) == 4u);
    // bsr rax, rcx (48 0F BD C1)
    REQUIRE(run_scan({0x48, 0x0F, 0xBD, 0xC1}, 0xDEAD,
                     0x8000000000000000ull) == 63u);
    // src == 0 preserves dst (zero-extended low half).
    REQUIRE(run_scan({0x48, 0x0F, 0xBC, 0xC1}, 0xDEAD, 0) == 0xDEADu);
    // 32-bit form: bsr eax, ecx (0F BD C1).
    REQUIRE(run_scan({0x0F, 0xBD, 0xC1}, 0, 0x80000000ull) == 31u);
}

TEST_CASE("e2e: POPCNT 32-bit form counts and sets ZF") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    translator::Translator tx;
    // popcnt eax, ecx; jz +N — ECX = 0xF0F0 → 8, ZF clear.
    static const std::vector<std::uint8_t> body{
        0xF3, 0x0F, 0xB8, 0xC1,                    // popcnt eax, ecx
        0x74, 0x0B,                                 // jz +11 (to movabs rdi,2)
        0x48, 0xBF, 0x01, 0, 0, 0, 0, 0, 0, 0,      // movabs rdi, 1
        0xC3,
        0x48, 0xBF, 0x02, 0, 0, 0, 0, 0, 0, 0,      // movabs rdi, 2
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= body.size()) return {};
        return std::span<const std::uint8_t>(body.data() + off,
                                             body.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    disp.state()[ir::Gpr::Rcx] = 0xF0F0u;
    auto r = disp.run(0x4000, 8);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state()[ir::Gpr::Rax] == 8u);
    REQUIRE(disp.state()[ir::Gpr::Rdi] == 1u);  // ZF clear: jz not taken
}

TEST_CASE("e2e: VZEROUPPER / VZEROALL — F2 AVX transition hygiene") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    auto run_vzero = [](std::uint8_t vex_byte2) {
        translator::Translator tx;
        std::vector<std::uint8_t> body{0xC5, vex_byte2, 0x77, 0xC3};
        auto reader =
            [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
            if (pc < 0x4000ull) return {};
            const std::size_t off =
                static_cast<std::size_t>(pc - 0x4000ull);
            if (off >= body.size()) return {};
            return std::span<const std::uint8_t>(body.data() + off,
                                                 body.size() - off);
        };
        runtime::Dispatcher disp{tx, reader};
        disp.install_halt_return_stack();
        for (std::size_t i = 0; i < 16; ++i) {
            disp.state().xmm[i].lo = 0x1111'0000ull + i;
            disp.state().xmm[i].hi = 0x2222'0000ull + i;
            disp.state().ymm_hi[i].lo = 0x3333'0000ull + i;
            disp.state().ymm_hi[i].hi = 0x4444'0000ull + i;
        }
        auto r = disp.run(0x4000, 4);
        REQUIRE(r.exit == runtime::DispatchExit::Halted);
        runtime::CpuStateFrame out = disp.state();
        return out;
    };

    SECTION("VZEROUPPER clears ymm_hi, preserves xmm") {
        auto s = run_vzero(0xF8);  // L=0
        for (std::size_t i = 0; i < 16; ++i) {
            REQUIRE(s.ymm_hi[i].lo == 0u);
            REQUIRE(s.ymm_hi[i].hi == 0u);
            REQUIRE(s.xmm[i].lo == 0x1111'0000ull + i);
            REQUIRE(s.xmm[i].hi == 0x2222'0000ull + i);
        }
    }
    SECTION("VZEROALL clears the full ymm file") {
        auto s = run_vzero(0xFC);  // L=1
        for (std::size_t i = 0; i < 16; ++i) {
            REQUIRE(s.ymm_hi[i].lo == 0u);
            REQUIRE(s.ymm_hi[i].hi == 0u);
            REQUIRE(s.xmm[i].lo == 0u);
            REQUIRE(s.xmm[i].hi == 0u);
        }
    }
}

TEST_CASE("e2e: VPMOVZXBW ymm0, xmm1 — F2-IR-005 byte→word zero-extend ymm") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // VPMOVZXBW: 66 0F 38 30 /r. C4 byte1=0xE2, byte2: W=0 vvvv=1111 L=1 pp=01 → 0x7D.
    // Reads 16 source bytes from xmm1, zero-extends each to a word
    // (16 result words spread across ymm0).
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC4, 0xE2, 0x7D, 0x30, 0xC1,
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    // xmm1 bytes: 0x01 .. 0x10 across all 16 lanes.
    disp.state().xmm[1].lo = 0x0807060504030201ULL;
    disp.state().xmm[1].hi = 0x100F0E0D0C0B0A09ULL;
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    // ymm0 should contain bytes 1..16 each zero-extended to a word.
    // Lower 128 (xmm[0]): words 0x0001 0x0002 0x0003 0x0004 (lo) 0x0005 0x0006 0x0007 0x0008 (hi)
    // Upper 128 (ymm_hi[0]): 0x0009..0x000C / 0x000D..0x0010
    auto pack4w = [](std::uint16_t a, std::uint16_t b,
                     std::uint16_t c, std::uint16_t d) -> std::uint64_t {
        return static_cast<std::uint64_t>(a)
            | (static_cast<std::uint64_t>(b) << 16)
            | (static_cast<std::uint64_t>(c) << 32)
            | (static_cast<std::uint64_t>(d) << 48);
    };
    REQUIRE(disp.state().xmm[0].lo    == pack4w(0x0001, 0x0002, 0x0003, 0x0004));
    REQUIRE(disp.state().xmm[0].hi    == pack4w(0x0005, 0x0006, 0x0007, 0x0008));
    REQUIRE(disp.state().ymm_hi[0].lo == pack4w(0x0009, 0x000A, 0x000B, 0x000C));
    REQUIRE(disp.state().ymm_hi[0].hi == pack4w(0x000D, 0x000E, 0x000F, 0x0010));
}

TEST_CASE("e2e: VPMOVMSKB rax, ymm0 — F2-IR-005 32-bit MSB extract from ymm") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // 66 0F D7 /r → PMOVMSKB. C5 byte1: pp=01, L=1, vvvv=1111 → 0xFD.
    // ModRM C0 = 11_000_000 → reg=rax (GPR dst), rm=ymm0 (xmm src).
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC5, 0xFD, 0xD7, 0xC0,
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    // Set bytes with MSB pattern: alternating 0x80 / 0x00 across the
    // 32 bytes. The mask should be 0x55555555 (bit i set iff lane i has
    // MSB set; even-lane → 0x80 → MSB set).
    disp.state().xmm[0].lo    = 0x0080008000800080ULL;  // bytes: 80 00 80 00 80 00 80 00
    disp.state().xmm[0].hi    = 0x0080008000800080ULL;
    disp.state().ymm_hi[0].lo = 0x0080008000800080ULL;
    disp.state().ymm_hi[0].hi = 0x0080008000800080ULL;
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    // Mask: lane 0 = 0x80 → bit 0 set; lane 1 = 0x00 → bit 1 clear; ...
    // Pattern 0x80 at even lanes (0, 2, 4, ...) → bit 0, 2, 4, ... set.
    // 32 bytes → 32-bit mask = 0b01010101_01010101_01010101_01010101 = 0x55555555.
    REQUIRE(disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rax)] == 0x55555555ULL);
}

TEST_CASE("e2e: VMOVDQA ymm0, ymm1 — F2-IR-005 256-bit register-to-register move") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // 66 0F 6F /r → MOVDQA. C5 byte1: pp=01, L=1, vvvv=1111 → 0xFD.
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC5, 0xFD, 0x6F, 0xC1,
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    disp.state().xmm[1].lo    = 0x1111111111111111ULL;
    disp.state().xmm[1].hi    = 0x2222222222222222ULL;
    disp.state().ymm_hi[1].lo = 0x3333333333333333ULL;
    disp.state().ymm_hi[1].hi = 0x4444444444444444ULL;
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    // ymm0 ← ymm1 — full 256-bit copy.
    REQUIRE(disp.state().xmm[0].lo    == 0x1111111111111111ULL);
    REQUIRE(disp.state().xmm[0].hi    == 0x2222222222222222ULL);
    REQUIRE(disp.state().ymm_hi[0].lo == 0x3333333333333333ULL);
    REQUIRE(disp.state().ymm_hi[0].hi == 0x4444444444444444ULL);
}

TEST_CASE("e2e: VPSHUFD ymm0, ymm1, 0x1B — F2-IR-005 32-bit lane reverse ymm") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // VPSHUFD = 66 0F 70. C5 byte1: pp=01, L=1, vvvv=1111 → 0xFD.
    // Imm 0x1B = 0b00_01_10_11 → reverse the 4 lanes (per 128-bit half).
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC5, 0xFD, 0x70, 0xC1, 0x1B,
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack4i = [](std::uint32_t a, std::uint32_t b) -> std::uint64_t {
        return static_cast<std::uint64_t>(a) | (static_cast<std::uint64_t>(b) << 32);
    };
    // ymm1 = [1, 2, 3, 4 | 5, 6, 7, 8]
    disp.state().xmm[1].lo    = pack4i(1u, 2u);
    disp.state().xmm[1].hi    = pack4i(3u, 4u);
    disp.state().ymm_hi[1].lo = pack4i(5u, 6u);
    disp.state().ymm_hi[1].hi = pack4i(7u, 8u);
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    // Per-half reverse: low half [4, 3, 2, 1], high half [8, 7, 6, 5].
    REQUIRE(disp.state().xmm[0].lo    == pack4i(4u, 3u));
    REQUIRE(disp.state().xmm[0].hi    == pack4i(2u, 1u));
    REQUIRE(disp.state().ymm_hi[0].lo == pack4i(8u, 7u));
    REQUIRE(disp.state().ymm_hi[0].hi == pack4i(6u, 5u));
}

TEST_CASE("e2e: VPABSB ymm0, ymm1 — F2-IR-005 byte abs ymm") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // VPABSB = 66 0F 38 1C /r. C4 byte1=0xE2, byte2: W=0 vvvv=1111 L=1 pp=01 → 0x7D.
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC4, 0xE2, 0x7D, 0x1C, 0xC1,
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    // bytes: -1, -2, -3, ...; abs should yield 1, 2, 3, ...
    disp.state().xmm[1].lo    = 0xFFFEFDFCFBFAF9F8ULL;  // -1, -2, -3, ..., -8
    disp.state().xmm[1].hi    = 0xF7F6F5F4F3F2F1F0ULL;  // -9, ..., -16
    disp.state().ymm_hi[1].lo = 0x0102030405060708ULL;  // 1, 2, 3, ..., 8 (positive untouched)
    disp.state().ymm_hi[1].hi = 0x80FF7F8081827F00ULL;  // mix: -128, -1, +127, ...
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[0].lo == 0x0102030405060708ULL);
    REQUIRE(disp.state().xmm[0].hi == 0x090A0B0C0D0E0F10ULL);
    REQUIRE(disp.state().ymm_hi[0].lo == 0x0102030405060708ULL);
    // -128 (0x80) abs is +128, but +128 doesn't fit signed-i8 → wraps back to -128 (0x80).
    // -1 (0xFF) → 1; +127 (0x7F) → 127; -128 (0x80) → 128 wraps to 0x80;
    // -127 (0x81) → 127 (0x7F); -126 (0x82) → 126 (0x7E); +127 (0x7F) → 127 (0x7F); 0 → 0.
    REQUIRE(disp.state().ymm_hi[0].hi == 0x80017F807F7E7F00ULL);
}

TEST_CASE("e2e: VPMULLD ymm2, ymm0, ymm1 — F2-IR-005 SSE4.1 32-bit mul ymm") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // VPMULLD = 66 0F 38 40 /r. C4 byte1=0xE2 (mmmmm=2), byte2: W=0
    // vvvv=inv(0)=1111 L=1 pp=01 → 0x7D.
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC4, 0xE2, 0x7D, 0x40, 0xD1,
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack4i = [](std::uint32_t a, std::uint32_t b) -> std::uint64_t {
        return static_cast<std::uint64_t>(a) | (static_cast<std::uint64_t>(b) << 32);
    };
    disp.state().xmm[0].lo    = pack4i(2u, 3u);
    disp.state().xmm[0].hi    = pack4i(4u, 5u);
    disp.state().ymm_hi[0].lo = pack4i(6u, 7u);
    disp.state().ymm_hi[0].hi = pack4i(8u, 9u);
    disp.state().xmm[1].lo    = pack4i(10u, 100u);
    disp.state().xmm[1].hi    = pack4i(1000u, 10000u);
    disp.state().ymm_hi[1].lo = pack4i(2u, 4u);
    disp.state().ymm_hi[1].hi = pack4i(8u, 16u);
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[2].lo    == pack4i(20u, 300u));
    REQUIRE(disp.state().xmm[2].hi    == pack4i(4000u, 50000u));
    REQUIRE(disp.state().ymm_hi[2].lo == pack4i(12u, 28u));
    REQUIRE(disp.state().ymm_hi[2].hi == pack4i(64u, 144u));
}

TEST_CASE("e2e: VPADDSB ymm2, ymm0, ymm1 — F2-IR-005 saturated byte add ymm") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // VEX C5 [byte1] EC D1 — pp=01 (66), L=1 → byte1 = 0xFD
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC5, 0xFD, 0xEC, 0xD1,
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    // Saturating signed byte add: positive saturation at +127, negative at -128.
    // Use bytes that overflow to test sat clamp.
    disp.state().xmm[0].lo    = 0x7F7F7F7F7F7F7F7FULL;  // all +127
    disp.state().xmm[0].hi    = 0x8080808080808080ULL;  // all -128
    disp.state().ymm_hi[0].lo = 0x0102030405060708ULL;
    disp.state().ymm_hi[0].hi = 0xF1F2F3F4F5F6F7F8ULL;
    disp.state().xmm[1].lo    = 0x0101010101010101ULL;  // all +1 → +127+1=+128 saturates to +127
    disp.state().xmm[1].hi    = 0xFFFFFFFFFFFFFFFFULL;  // all -1 → -128+(-1)=-129 saturates to -128
    disp.state().ymm_hi[1].lo = 0x0101010101010101ULL;
    disp.state().ymm_hi[1].hi = 0x0101010101010101ULL;
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[2].lo    == 0x7F7F7F7F7F7F7F7FULL);  // saturated +127
    REQUIRE(disp.state().xmm[2].hi    == 0x8080808080808080ULL);  // saturated -128
    // ymm_hi: 0x01..08 + 0x01 = 0x02..09; 0xF1..F8 + 0x01 = 0xF2..F9.
    REQUIRE(disp.state().ymm_hi[2].lo == 0x0203040506070809ULL);
    REQUIRE(disp.state().ymm_hi[2].hi == 0xF2F3F4F5F6F7F8F9ULL);
}

TEST_CASE("e2e: VPMULLW ymm2, ymm0, ymm1 — F2-IR-005 16-bit lane multiply ymm") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // VPMULLW = 66 0F D5. C5 [byte1] D5 D1, byte1 = 0xFD.
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC5, 0xFD, 0xD5, 0xD1,
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack4w = [](std::uint16_t a, std::uint16_t b,
                     std::uint16_t c, std::uint16_t d) -> std::uint64_t {
        return static_cast<std::uint64_t>(a)
            | (static_cast<std::uint64_t>(b) << 16)
            | (static_cast<std::uint64_t>(c) << 32)
            | (static_cast<std::uint64_t>(d) << 48);
    };
    disp.state().xmm[0].lo    = pack4w(2, 3, 4, 5);
    disp.state().xmm[0].hi    = pack4w(6, 7, 8, 9);
    disp.state().ymm_hi[0].lo = pack4w(10, 11, 12, 13);
    disp.state().ymm_hi[0].hi = pack4w(14, 15, 16, 17);
    disp.state().xmm[1].lo    = pack4w(10, 10, 10, 10);
    disp.state().xmm[1].hi    = pack4w(10, 10, 10, 10);
    disp.state().ymm_hi[1].lo = pack4w(10, 10, 10, 10);
    disp.state().ymm_hi[1].hi = pack4w(10, 10, 10, 10);
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[2].lo    == pack4w(20, 30, 40, 50));
    REQUIRE(disp.state().xmm[2].hi    == pack4w(60, 70, 80, 90));
    REQUIRE(disp.state().ymm_hi[2].lo == pack4w(100, 110, 120, 130));
    REQUIRE(disp.state().ymm_hi[2].hi == pack4w(140, 150, 160, 170));
}

TEST_CASE("e2e: MUL r/m64 produces RDX:RAX 128-bit product (F2-BK-007)") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // 48 F7 E3 → MUL rbx (rdx:rax = rax * rbx)
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0x48, 0xF7, 0xE3,
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    // 0xFFFFFFFFFFFFFFFF * 2 = 0x1FFFFFFFFFFFFFFFE → low = 0xFFFFFFFFFFFFFFFE,
    //   hi = 0x0000000000000001
    disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rax)] = 0xFFFFFFFFFFFFFFFFULL;
    disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rbx)] = 2ULL;
    disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rdx)] = 0xDEADBEEFULL;
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rax)] ==
            0xFFFFFFFFFFFFFFFEULL);
    REQUIRE(disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rdx)] == 1ULL);
}

TEST_CASE("e2e: DIV r/m64 → quotient in RAX, remainder in RDX (F2-BK-007)") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // 48 F7 F3 → DIV rbx (RAX = RAX / RBX, RDX = RAX % RBX)
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0x48, 0xF7, 0xF3,
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rax)] = 100ULL;
    disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rbx)] = 7ULL;
    disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rdx)] = 0xDEADBEEFULL;
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    // 100 / 7 = 14, 100 % 7 = 2
    REQUIRE(disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rax)] == 14ULL);
    REQUIRE(disp.state().gpr[static_cast<std::size_t>(ir::Gpr::Rdx)] == 2ULL);
}

TEST_CASE("e2e: VFMADDSUB132PS xmm2, xmm0, xmm1 — F2-IR-006 alternating add/sub") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // VFMADDSUB132PS: 132 form, opcode 0x96, packed PS (W=0).
    //   xmm2 = (xmm2 * xmm1) ± xmm0  with even lane = SUB, odd = ADD.
    // C4 byte1 = 0xE2, byte2: W=0 vvvv=0xF L=0 pp=01 → 0x79
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC4, 0xE2, 0x79, 0x96, 0xD1,
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack2f = [](float a, float b) -> std::uint64_t {
        std::uint32_t aa, bb;
        std::memcpy(&aa, &a, 4); std::memcpy(&bb, &b, 4);
        return static_cast<std::uint64_t>(aa) | (static_cast<std::uint64_t>(bb) << 32);
    };
    // xmm2 = [10, 20, 30, 40], xmm1 = [2, 3, 4, 5], xmm0 = [1, 1, 1, 1]
    // 132: lane[i] = (xmm2[i] * xmm1[i]) ± xmm0[i]
    //   lane 0 (SUB): 10*2 - 1 = 19
    //   lane 1 (ADD): 20*3 + 1 = 61
    //   lane 2 (SUB): 30*4 - 1 = 119
    //   lane 3 (ADD): 40*5 + 1 = 201
    disp.state().xmm[0].lo = pack2f(1.0f, 1.0f);
    disp.state().xmm[0].hi = pack2f(1.0f, 1.0f);
    disp.state().xmm[1].lo = pack2f(2.0f, 3.0f);
    disp.state().xmm[1].hi = pack2f(4.0f, 5.0f);
    disp.state().xmm[2].lo = pack2f(10.0f, 20.0f);
    disp.state().xmm[2].hi = pack2f(30.0f, 40.0f);
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[2].lo == pack2f(19.0f, 61.0f));
    REQUIRE(disp.state().xmm[2].hi == pack2f(119.0f, 201.0f));
}

TEST_CASE("e2e: VFMSUBADD132PS xmm2, xmm0, xmm1 — F2-IR-006 alternating sub/add") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // VFMSUBADD132PS: opcode 0x97, packed PS.
    //   xmm2 = (xmm2 * xmm1) ± xmm0  with even lane = ADD, odd = SUB.
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC4, 0xE2, 0x79, 0x97, 0xD1,
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack2f = [](float a, float b) -> std::uint64_t {
        std::uint32_t aa, bb;
        std::memcpy(&aa, &a, 4); std::memcpy(&bb, &b, 4);
        return static_cast<std::uint64_t>(aa) | (static_cast<std::uint64_t>(bb) << 32);
    };
    disp.state().xmm[0].lo = pack2f(1.0f, 1.0f);
    disp.state().xmm[0].hi = pack2f(1.0f, 1.0f);
    disp.state().xmm[1].lo = pack2f(2.0f, 3.0f);
    disp.state().xmm[1].hi = pack2f(4.0f, 5.0f);
    disp.state().xmm[2].lo = pack2f(10.0f, 20.0f);
    disp.state().xmm[2].hi = pack2f(30.0f, 40.0f);
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    // Even lane = ADD, odd = SUB.
    //   lane 0 (ADD): 10*2 + 1 = 21
    //   lane 1 (SUB): 20*3 - 1 = 59
    //   lane 2 (ADD): 30*4 + 1 = 121
    //   lane 3 (SUB): 40*5 - 1 = 199
    REQUIRE(disp.state().xmm[2].lo == pack2f(21.0f, 59.0f));
    REQUIRE(disp.state().xmm[2].hi == pack2f(121.0f, 199.0f));
}

TEST_CASE("e2e: VFMADD231SS xmm2, xmm0, xmm1 — F2-IR-006 scalar single FMA") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // VFMADD231SS xmm2, xmm0, xmm1 → low lane: xmm2 = (xmm0 * xmm1) + xmm2
    // Upper lanes preserved from xmm2 (the destination).
    // Opcode 0xB9 (231 odd low-nibble = scalar). C4 byte2: W=0 vvvv=0xF L=0 pp=01 → 0x79.
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC4, 0xE2, 0x79, 0xB9, 0xD1,
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack2f = [](float a, float b) -> std::uint64_t {
        std::uint32_t aa, bb;
        std::memcpy(&aa, &a, 4); std::memcpy(&bb, &b, 4);
        return static_cast<std::uint64_t>(aa) | (static_cast<std::uint64_t>(bb) << 32);
    };
    // xmm0 lane 0 = 3.0, xmm1 lane 0 = 4.0, xmm2 lane 0 = 1.0.
    // result xmm2 lane 0 = (3.0 * 4.0) + 1.0 = 13.0.
    // Upper 96 bits of xmm2 must be preserved.
    disp.state().xmm[0].lo = pack2f(3.0f, 99.0f);
    disp.state().xmm[0].hi = pack2f(98.0f, 97.0f);
    disp.state().xmm[1].lo = pack2f(4.0f, 88.0f);
    disp.state().xmm[1].hi = pack2f(87.0f, 86.0f);
    const std::uint64_t orig_lo_upper_lane = pack2f(1.0f, 77.5f);
    disp.state().xmm[2].lo = orig_lo_upper_lane;
    disp.state().xmm[2].hi = 0xCAFEBABE12345678ULL;
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    // Low lane = 13.0; upper 32 bits of xmm[2].lo (= 77.5f) preserved;
    // hi 64 bits preserved entirely.
    REQUIRE(disp.state().xmm[2].lo == pack2f(13.0f, 77.5f));
    REQUIRE(disp.state().xmm[2].hi == 0xCAFEBABE12345678ULL);
}

TEST_CASE("e2e: VFMSUB132SD xmm2, xmm0, xmm1 — F2-IR-006 scalar double, 132 form") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // VFMSUB132SD xmm2, xmm0, xmm1: low lane: xmm2 = (xmm2 * xmm1) - xmm0
    // Opcode 0x9B (132 odd = scalar). C4 byte2: W=1 (SD) vvvv=0xF L=0 pp=01 → 0xF9.
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC4, 0xE2, 0xF9, 0x9B, 0xD1,
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto u64_of_double = [](double v) -> std::uint64_t {
        std::uint64_t bits; std::memcpy(&bits, &v, 8); return bits;
    };
    // xmm0 lane 0 = 5.0, xmm1 lane 0 = 4.0, xmm2 lane 0 = 3.0.
    // 132: low lane result = (3.0 * 4.0) - 5.0 = 7.0.
    // Upper 64 bits of xmm2 must be preserved.
    disp.state().xmm[0].lo = u64_of_double(5.0);
    disp.state().xmm[1].lo = u64_of_double(4.0);
    disp.state().xmm[2].lo = u64_of_double(3.0);
    disp.state().xmm[2].hi = 0x123456789ABCDEF0ULL;  // sentinel
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[2].lo == u64_of_double(7.0));
    REQUIRE(disp.state().xmm[2].hi == 0x123456789ABCDEF0ULL);  // preserved
}

TEST_CASE("e2e: VFMADD231PS ymm2, ymm0, ymm1 — F2-IR-006 ymm 256-bit FMA") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // VEX C4 byte1 byte2 B8 D1
    //   byte1: R̅ X̅ B̅ mmmmm = 1_1_1_00010 = 0xE2 (mmmmm=2 → 0F 38)
    //   byte2: W vvvv L pp     = 0_1111_1_01 = 0x7D (W=0 PS, vvvv=0xF, L=1, pp=01)
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC4, 0xE2, 0x7D, 0xB8, 0xD1,
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack2f = [](float a, float b) -> std::uint64_t {
        std::uint32_t aa, bb;
        std::memcpy(&aa, &a, 4); std::memcpy(&bb, &b, 4);
        return static_cast<std::uint64_t>(aa) | (static_cast<std::uint64_t>(bb) << 32);
    };
    // ymm0 = [2..9], ymm1 = [10,20,30,40,50,60,70,80], ymm2 = [1,1,1,1,1,1,1,1]
    // result = (ymm0*ymm1) + ymm2 = [21, 61, 121, 201, 301, 421, 561, 721]
    disp.state().xmm[0].lo    = pack2f(2.0f, 3.0f);
    disp.state().xmm[0].hi    = pack2f(4.0f, 5.0f);
    disp.state().ymm_hi[0].lo = pack2f(6.0f, 7.0f);
    disp.state().ymm_hi[0].hi = pack2f(8.0f, 9.0f);
    disp.state().xmm[1].lo    = pack2f(10.0f, 20.0f);
    disp.state().xmm[1].hi    = pack2f(30.0f, 40.0f);
    disp.state().ymm_hi[1].lo = pack2f(50.0f, 60.0f);
    disp.state().ymm_hi[1].hi = pack2f(70.0f, 80.0f);
    disp.state().xmm[2].lo    = pack2f(1.0f, 1.0f);
    disp.state().xmm[2].hi    = pack2f(1.0f, 1.0f);
    disp.state().ymm_hi[2].lo = pack2f(1.0f, 1.0f);
    disp.state().ymm_hi[2].hi = pack2f(1.0f, 1.0f);
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[2].lo    == pack2f(21.0f, 61.0f));
    REQUIRE(disp.state().xmm[2].hi    == pack2f(121.0f, 201.0f));
    REQUIRE(disp.state().ymm_hi[2].lo == pack2f(301.0f, 421.0f));
    REQUIRE(disp.state().ymm_hi[2].hi == pack2f(561.0f, 721.0f));
}

TEST_CASE("e2e: VFNMADD231PS xmm2, xmm0, xmm1 — F2-IR-006 a - b*c") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // VFNMADD231PS: xmm2 = -(xmm0 * xmm1) + xmm2 = xmm2 - (xmm0*xmm1)
    // Opcode 0xBC. byte2 = 0x79 (W=0, vvvv=0xF, L=0, pp=01).
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC4, 0xE2, 0x79, 0xBC, 0xD1,
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack2f = [](float a, float b) -> std::uint64_t {
        std::uint32_t aa, bb;
        std::memcpy(&aa, &a, 4); std::memcpy(&bb, &b, 4);
        return static_cast<std::uint64_t>(aa) | (static_cast<std::uint64_t>(bb) << 32);
    };
    // xmm0 = [2, 3, 4, 5]   xmm1 = [10, 20, 30, 40]   xmm2 = [100, 200, 300, 400]
    // result = xmm2 - (xmm0*xmm1) = [80, 140, 180, 200]
    disp.state().xmm[0].lo = pack2f(2.0f, 3.0f);
    disp.state().xmm[0].hi = pack2f(4.0f, 5.0f);
    disp.state().xmm[1].lo = pack2f(10.0f, 20.0f);
    disp.state().xmm[1].hi = pack2f(30.0f, 40.0f);
    disp.state().xmm[2].lo = pack2f(100.0f, 200.0f);
    disp.state().xmm[2].hi = pack2f(300.0f, 400.0f);
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[2].lo == pack2f(80.0f, 140.0f));
    REQUIRE(disp.state().xmm[2].hi == pack2f(180.0f, 200.0f));
}

TEST_CASE("e2e: VFMADD132PD xmm2, xmm0, xmm1 — F2-IR-006 132-form FMA double") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // VFMADD132PD xmm2, xmm0, xmm1 → xmm2 = (xmm2 * xmm1) + xmm0
    //   C4 byte1 = 0xE2 (R̅=1 X̅=1 B̅=1 mmmmm=2)
    //   C4 byte2: W=1 (PD), vvvv=inverted(0)=1111, L=0, pp=01
    //          → 1_1111_0_01 = 0xF9
    //   opcode 0x98 (even low-nibble = packed; W=1 → PD)
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC4, 0xE2, 0xF9, 0x98, 0xD1,
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto u64_of_double = [](double v) -> std::uint64_t {
        std::uint64_t bits; std::memcpy(&bits, &v, 8); return bits;
    };
    // xmm0 = [10, 20]   xmm1 = [3, 4]   xmm2 = [5, 6]
    // 132 form: xmm2 = (xmm2 * xmm1) + xmm0 = [(5*3)+10, (6*4)+20] = [25, 44]
    disp.state().xmm[0].lo = u64_of_double(10.0);
    disp.state().xmm[0].hi = u64_of_double(20.0);
    disp.state().xmm[1].lo = u64_of_double(3.0);
    disp.state().xmm[1].hi = u64_of_double(4.0);
    disp.state().xmm[2].lo = u64_of_double(5.0);
    disp.state().xmm[2].hi = u64_of_double(6.0);
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[2].lo == u64_of_double(25.0));
    REQUIRE(disp.state().xmm[2].hi == u64_of_double(44.0));
}

TEST_CASE("e2e: VSHUFPS ymm2, ymm0, ymm1, 0x1B — F2-IR-005 AVX-256 lane reverse") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // VEX C5 [byte1] C6 D1 ib, pp=00 (PS), L=1 → byte1 = 0xFC.
    // Imm 0x1B = 0b00_01_10_11: dst[0]=src1[3], dst[1]=src1[2],
    //                            dst[2]=src2[1], dst[3]=src2[0].
    // Per-128-bit-lane: same control byte applies to both halves.
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC5, 0xFC, 0xC6, 0xD1, 0x1B,  // vshufps ymm2, ymm0, ymm1, 0x1B
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto pack4i = [](std::uint32_t a, std::uint32_t b) -> std::uint64_t {
        return static_cast<std::uint64_t>(a) | (static_cast<std::uint64_t>(b) << 32);
    };
    // ymm0 lanes 1..4 in low 128, 5..8 in high 128.
    disp.state().xmm[0].lo    = pack4i(1u, 2u);
    disp.state().xmm[0].hi    = pack4i(3u, 4u);
    disp.state().ymm_hi[0].lo = pack4i(5u, 6u);
    disp.state().ymm_hi[0].hi = pack4i(7u, 8u);
    // ymm1 lanes 11..14 in low, 15..18 in high.
    disp.state().xmm[1].lo    = pack4i(11u, 12u);
    disp.state().xmm[1].hi    = pack4i(13u, 14u);
    disp.state().ymm_hi[1].lo = pack4i(15u, 16u);
    disp.state().ymm_hi[1].hi = pack4i(17u, 18u);
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    // Per-half reverse: low half gets [4, 3, 12, 11], high gets [8, 7, 16, 15].
    REQUIRE(disp.state().xmm[2].lo    == pack4i(4u, 3u));
    REQUIRE(disp.state().xmm[2].hi    == pack4i(12u, 11u));
    REQUIRE(disp.state().ymm_hi[2].lo == pack4i(8u, 7u));
    REQUIRE(disp.state().ymm_hi[2].hi == pack4i(16u, 15u));
}

TEST_CASE("e2e: VMULPD ymm2, ymm0, ymm1 — F2-IR-005 AVX-256 packed double mul") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    // VEX C5 [byte1] 59 D1, pp = 01 (66 mandatory → PD), L = 1
    //   byte1 = 1_1111_1_01 = 0xFD
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0xC5, 0xFD, 0x59, 0xD1,  // vmulpd ymm2, ymm0, ymm1
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    auto u64_of_double = [](double v) -> std::uint64_t {
        std::uint64_t bits; std::memcpy(&bits, &v, 8); return bits;
    };
    // ymm0 = [2.0, 3.0, 4.0, 5.0]   ymm1 = [10.0, 20.0, 30.0, 40.0]
    disp.state().xmm[0].lo    = u64_of_double(2.0);
    disp.state().xmm[0].hi    = u64_of_double(3.0);
    disp.state().ymm_hi[0].lo = u64_of_double(4.0);
    disp.state().ymm_hi[0].hi = u64_of_double(5.0);
    disp.state().xmm[1].lo    = u64_of_double(10.0);
    disp.state().xmm[1].hi    = u64_of_double(20.0);
    disp.state().ymm_hi[1].lo = u64_of_double(30.0);
    disp.state().ymm_hi[1].hi = u64_of_double(40.0);
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state().xmm[2].lo    == u64_of_double(20.0));
    REQUIRE(disp.state().xmm[2].hi    == u64_of_double(60.0));
    REQUIRE(disp.state().ymm_hi[2].lo == u64_of_double(120.0));
    REQUIRE(disp.state().ymm_hi[2].hi == u64_of_double(200.0));
}

TEST_CASE("e2e: AESKEYGENASSIST xmm0, xmm1, 0x1b — F2-IR-058") {
    if constexpr (!is_arm64) { SUCCEED("skipped on non-ARM64 host"); return; }
    if (!sha_e2e::host_has_aes_crypto()) {
        SUCCEED("ARM64 host lacks AES crypto extensions");
        return;
    }
    translator::Translator tx;
    std::vector<std::uint8_t> code{
        0x66, 0x0F, 0x3A, 0xDF, 0xC1, 0x1B,  // aeskeygenassist xmm0, xmm1, 0x1b
        0xC3,
    };
    auto reader = [&](std::uint64_t pc) -> std::span<const std::uint8_t> {
        if (pc < 0x4000ull) return {};
        const std::size_t off = static_cast<std::size_t>(pc - 0x4000ull);
        if (off >= code.size()) return {};
        return std::span<const std::uint8_t>(code.data() + off,
                                             code.size() - off);
    };
    runtime::Dispatcher disp{tx, reader};
    disp.install_halt_return_stack();
    // src = 00 11 22 33 44 55 66 77 88 99 aa bb cc dd ee ff
    disp.state().xmm[1].lo = 0x7766554433221100ULL;
    disp.state().xmm[1].hi = 0xffeeddccbbaa9988ULL;
    auto r = disp.run(0x4000, 100);
    INFO("dispatch: " << r.message);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    // Expected from native x86 AES-NI:
    // 1b fc 33 f5 e7 33 f5 1b 4b c1 28 16 da 28 16 4b
    REQUIRE(disp.state().xmm[0].lo == 0x1bf533e7f533fc1bULL);
    REQUIRE(disp.state().xmm[0].hi == 0x4b1628da1628c14bULL);
}
