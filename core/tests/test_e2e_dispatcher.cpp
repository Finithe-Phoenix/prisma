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
    // 0x0000_0000_0000_0100 → leading zeros = 55, trailing zeros = 8.
    disp.state()[ir::Gpr::Rcx] = 0x0000000000000100ULL;
    disp.state()[ir::Gpr::Rdx] = 0x0000000000000100ULL;
    auto r = disp.run(0x4000, 100);
    REQUIRE(r.exit == runtime::DispatchExit::Halted);
    REQUIRE(disp.state()[ir::Gpr::Rax] == 55u);
    REQUIRE(disp.state()[ir::Gpr::Rbx] == 8u);
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
    REQUIRE(d1.run(0x4000, 100).exit == runtime::DispatchExit::Halted);
    REQUIRE(tx.stats().cache_misses == 1);
    REQUIRE(tx.stats().cache_hits == 0);

    // Second run on the *same* Translator — should hit the cache.
    runtime::Dispatcher d2{tx, reader};
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
