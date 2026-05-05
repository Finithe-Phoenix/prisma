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
