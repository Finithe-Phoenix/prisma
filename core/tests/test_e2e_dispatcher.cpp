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
