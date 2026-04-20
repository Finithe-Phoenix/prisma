// core/tests/test_emitter.cpp — integration tests for the vixl-backed emitter.
//
// Each test emits a small code sequence via the Emitter API, then compares
// the resulting bytes against the hand-rolled encoders in arm64_encoding.hpp.
// The hand-rolled encoders are the ground truth (each of them is tested
// against ARM ARM bit layouts in test_arm64_encoding.cpp); vixl is the
// implementation we actually ship. Matching the two is the safety net that
// catches regressions in either.

#include <catch2/catch_test_macros.hpp>
#include <cstring>

#include "prisma/emitter.hpp"
#include "prisma/arm64_encoding.hpp"

using namespace prisma;

namespace {

// Extract `count` little-endian 32-bit words from the emitter's bytes.
std::vector<std::uint32_t> as_words(backend::Emitter& em) {
    em.finalize();
    const auto bytes = em.code_bytes();
    std::vector<std::uint32_t> words;
    words.reserve(bytes.size() / 4);
    for (std::size_t i = 0; i + 4 <= bytes.size(); i += 4) {
        std::uint32_t w;
        std::memcpy(&w, bytes.data() + i, 4);
        words.push_back(w);
    }
    return words;
}

}  // namespace

TEST_CASE("Emitter: movz x0 #42 matches hand-rolled encoder") {
    backend::Emitter em;
    em.movz(arm64::Reg::X0, 42, 0);
    em.ret();

    const auto words = as_words(em);
    REQUIRE(words.size() == 2);

    REQUIRE(words[0] == arm64::movz_x(arm64::Reg::X0, 42, 0).raw);
    REQUIRE(words[0] == 0xD280'0540u);

    REQUIRE(words[1] == arm64::ret().raw);
    REQUIRE(words[1] == 0xD65F'03C0u);
}

TEST_CASE("Emitter: mov_imm64 of small value matches MOVZ with hw=0") {
    backend::Emitter em;
    em.mov_imm64(arm64::Reg::X0, 42);
    em.ret();

    const auto words = as_words(em);
    // A 16-bit-fitting immediate should collapse to a single movz.
    REQUIRE(words.size() == 2);
    REQUIRE(words[0] == 0xD280'0540u);
    REQUIRE(words[1] == 0xD65F'03C0u);
}

TEST_CASE("Emitter: mov_imm64 of wider value emits movz + movk sequence") {
    backend::Emitter em;
    em.mov_imm64(arm64::Reg::X0, 0x1234'5678u);  // 32-bit value

    const auto words = as_words(em);
    // Expect exactly 2 words: movz (low 16) + movk (high 16).
    REQUIRE(words.size() == 2);

    // First word is movz x0, #0x5678, lsl #0
    REQUIRE(words[0] == arm64::movz_x(arm64::Reg::X0, 0x5678u, 0).raw);
    // Second word is movk x0, #0x1234, lsl #16 — we don't hand-encode MOVK
    // yet, just verify the upper 16 bits of the instruction contain 0x1234
    // at the imm16 position ((word >> 5) & 0xFFFF).
    REQUIRE(((words[1] >> 5) & 0xFFFFu) == 0x1234u);
}

TEST_CASE("Emitter: disassemble produces readable output") {
    backend::Emitter em;
    em.movz(arm64::Reg::X0, 42, 0);
    em.ret();
    em.finalize();

    const std::string text = em.disassemble();
    // vixl's disassembler uses the canonical alias `mov` for `movz`
    // (both produce the same machine code for a 16-bit immediate).
    REQUIRE(text.find("mov") != std::string::npos);
    REQUIRE(text.find("ret") != std::string::npos);
    REQUIRE(text.find("x0") != std::string::npos);
    REQUIRE(text.find("0x2a") != std::string::npos);  // 42 in hex
}
