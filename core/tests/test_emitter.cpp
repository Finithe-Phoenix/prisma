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

// ---------------------------------------------------------------------------
// F1-BK-014 rol, F1-BK-015 clz/cls/rbit
// ---------------------------------------------------------------------------

TEST_CASE("Emitter: rol disassembles as neg + ror") {
    backend::Emitter em;
    em.rol(arm64::Reg::X0, arm64::Reg::X1, arm64::Reg::X2, arm64::Reg::X3);
    em.finalize();
    const std::string text = em.disassemble();
    REQUIRE(text.find("neg") != std::string::npos);
    REQUIRE(text.find("ror") != std::string::npos);
    REQUIRE(text.find("x3") != std::string::npos);  // tmp reg
}

TEST_CASE("Emitter: clz emits a clz instruction") {
    backend::Emitter em;
    em.clz(arm64::Reg::X0, arm64::Reg::X1);
    em.finalize();
    const std::string text = em.disassemble();
    REQUIRE(text.find("clz") != std::string::npos);
    REQUIRE(text.find("x0") != std::string::npos);
    REQUIRE(text.find("x1") != std::string::npos);
}

TEST_CASE("Emitter: cls emits a cls instruction") {
    backend::Emitter em;
    em.cls(arm64::Reg::X5, arm64::Reg::X6);
    em.finalize();
    const std::string text = em.disassemble();
    REQUIRE(text.find("cls") != std::string::npos);
}

TEST_CASE("Emitter: rbit emits an rbit instruction") {
    backend::Emitter em;
    em.rbit(arm64::Reg::X7, arm64::Reg::X8);
    em.finalize();
    const std::string text = em.disassemble();
    REQUIRE(text.find("rbit") != std::string::npos);
}

TEST_CASE("Emitter: width canonicalisation emits extend aliases") {
    backend::Emitter em;
    em.zero_extend(arm64::Reg::X0, arm64::Reg::X1, ir::OpSize::I8);
    em.zero_extend(arm64::Reg::X2, arm64::Reg::X3, ir::OpSize::I16);
    em.sign_extend(arm64::Reg::X4, arm64::Reg::X5, ir::OpSize::I8);
    em.sign_extend(arm64::Reg::X6, arm64::Reg::X7, ir::OpSize::I32);
    em.finalize();

    const std::string text = em.disassemble();
    REQUIRE(text.find("uxtb") != std::string::npos);
    REQUIRE(text.find("uxth") != std::string::npos);
    REQUIRE(text.find("sxtb") != std::string::npos);
    REQUIRE(text.find("sxtw") != std::string::npos);
}

TEST_CASE("Emitter: sized register copies use x86 GPR write semantics") {
    backend::Emitter em;
    em.mov_reg_reg(arm64::Reg::X0, arm64::Reg::X10, ir::OpSize::I32);
    em.store_reg_reg(arm64::Reg::X11, arm64::Reg::X1, ir::OpSize::I32);
    em.store_reg_reg(arm64::Reg::X12, arm64::Reg::X2, ir::OpSize::I8);
    em.store_reg_reg(arm64::Reg::X13, arm64::Reg::X3, ir::OpSize::I16);
    em.finalize();

    const std::string text = em.disassemble();
    REQUIRE(text.find("mov w0, w10") != std::string::npos);
    REQUIRE(text.find("mov w11, w1") != std::string::npos);
    REQUIRE(text.find("bfxil x12, x2, #0, #8") != std::string::npos);
    REQUIRE(text.find("bfxil x13, x3, #0, #16") != std::string::npos);
}

TEST_CASE("Emitter: x86 fences map to ARM barriers") {
    backend::Emitter em;
    em.fence(ir::FenceKind::Mfence);
    em.fence(ir::FenceKind::Lfence);
    em.fence(ir::FenceKind::Sfence);
    em.finalize();

    const std::string text = em.disassemble();
    REQUIRE(text.find("dmb ish")   != std::string::npos);
    REQUIRE(text.find("dsb ishld") != std::string::npos);
    REQUIRE(text.find("dmb ishst") != std::string::npos);
}

// ---------------------------------------------------------------------------
// F1-BK-011 mul/div multi-output
// ---------------------------------------------------------------------------

TEST_CASE("Emitter: umulh + mul pair covers 128-bit unsigned product") {
    backend::Emitter em;
    em.mul  (arm64::Reg::X0, arm64::Reg::X1, arm64::Reg::X2);  // low 64
    em.umulh(arm64::Reg::X3, arm64::Reg::X1, arm64::Reg::X2);  // high 64
    em.finalize();
    const std::string text = em.disassemble();
    REQUIRE(text.find("mul")   != std::string::npos);
    REQUIRE(text.find("umulh") != std::string::npos);
}

TEST_CASE("Emitter: smulh covers 128-bit signed high word") {
    backend::Emitter em;
    em.smulh(arm64::Reg::X0, arm64::Reg::X1, arm64::Reg::X2);
    em.finalize();
    REQUIRE(em.disassemble().find("smulh") != std::string::npos);
}

TEST_CASE("Emitter: udiv + msub computes quotient and remainder") {
    // q = n / m ; r = n - q*m
    backend::Emitter em;
    em.udiv(arm64::Reg::X0, arm64::Reg::X1, arm64::Reg::X2);   // x0 = x1 / x2
    em.msub(arm64::Reg::X3, arm64::Reg::X0, arm64::Reg::X2,    // x3 = x1 - x0*x2
            arm64::Reg::X1);
    em.finalize();
    const std::string text = em.disassemble();
    REQUIRE(text.find("udiv") != std::string::npos);
    REQUIRE(text.find("msub") != std::string::npos);
}

TEST_CASE("Emitter: sdiv emits sdiv") {
    backend::Emitter em;
    em.sdiv(arm64::Reg::X0, arm64::Reg::X1, arm64::Reg::X2);
    em.finalize();
    REQUIRE(em.disassemble().find("sdiv") != std::string::npos);
}

// ---------------------------------------------------------------------------
// F1-BK-016 atomic RMW via ldxr / stxr
// ---------------------------------------------------------------------------

TEST_CASE("Emitter: ldxr + stxr pair for a 64-bit RMW") {
    backend::Emitter em;
    em.ldxr (arm64::Reg::X0, arm64::Reg::X1, ir::OpSize::I64);
    em.stxr (arm64::Reg::X2, arm64::Reg::X3, arm64::Reg::X1, ir::OpSize::I64);
    em.finalize();
    const std::string text = em.disassemble();
    REQUIRE(text.find("ldxr") != std::string::npos);
    REQUIRE(text.find("stxr") != std::string::npos);
}

TEST_CASE("Emitter: ldaxr + stlxr pair is the acquire-release variant") {
    backend::Emitter em;
    em.ldaxr(arm64::Reg::X0, arm64::Reg::X1, ir::OpSize::I64);
    em.stlxr(arm64::Reg::X2, arm64::Reg::X3, arm64::Reg::X1, ir::OpSize::I64);
    em.finalize();
    const std::string text = em.disassemble();
    REQUIRE(text.find("ldaxr") != std::string::npos);
    REQUIRE(text.find("stlxr") != std::string::npos);
}

TEST_CASE("Emitter: ldxr on byte size emits ldxrb") {
    backend::Emitter em;
    em.ldxr(arm64::Reg::X0, arm64::Reg::X1, ir::OpSize::I8);
    em.finalize();
    REQUIRE(em.disassemble().find("ldxrb") != std::string::npos);
}

// ---------------------------------------------------------------------------
// F1-BK-017 LSE atomics
// ---------------------------------------------------------------------------

TEST_CASE("Emitter: casal emits cas with acquire-release ordering") {
    backend::Emitter em;
    em.casal(arm64::Reg::X0, arm64::Reg::X1, arm64::Reg::X2, ir::OpSize::I64);
    em.finalize();
    // vixl spells it `casal` in disassembly (sequential-consistent CAS).
    REQUIRE(em.disassemble().find("casal") != std::string::npos);
}

TEST_CASE("Emitter: ldaddal emits the LSE fetch-and-add") {
    backend::Emitter em;
    em.ldaddal(arm64::Reg::X0, arm64::Reg::X1, arm64::Reg::X2, ir::OpSize::I64);
    em.finalize();
    REQUIRE(em.disassemble().find("ldaddal") != std::string::npos);
}

TEST_CASE("Emitter: casal byte variant emits casalb") {
    backend::Emitter em;
    em.casal(arm64::Reg::X0, arm64::Reg::X1, arm64::Reg::X2, ir::OpSize::I8);
    em.finalize();
    REQUIRE(em.disassemble().find("casalb") != std::string::npos);
}

// ---------------------------------------------------------------------------
// F1-BK-005 label management
// ---------------------------------------------------------------------------

TEST_CASE("Emitter: forward branch to a later-bound label resolves") {
    backend::Emitter em;
    auto skip = em.create_label();
    em.movz(arm64::Reg::X0, 1, 0);
    em.branch(skip);                  // forward branch — vixl fixes up on bind
    em.movz(arm64::Reg::X0, 2, 0);    // dead code — skipped at runtime
    em.bind(skip);
    em.ret();
    em.finalize();

    const std::string text = em.disassemble();
    REQUIRE(text.find("b ") != std::string::npos);  // unconditional branch
}

TEST_CASE("Emitter: conditional branch uses the right cond suffix") {
    backend::Emitter em;
    auto target = em.create_label();
    em.cmp(arm64::Reg::X0, arm64::Reg::X1);
    em.branch_cc(target, ir::CondCode::Eq);
    em.ret();
    em.bind(target);
    em.ret();
    em.finalize();

    const std::string text = em.disassemble();
    // vixl disassembles b.eq as `b.eq` (with the cond suffix).
    REQUIRE(text.find("b.eq") != std::string::npos);
}

TEST_CASE("Emitter: backward branch to an already-bound label works") {
    backend::Emitter em;
    auto loop_top = em.create_label();
    em.bind(loop_top);
    em.sub(arm64::Reg::X0, arm64::Reg::X0, arm64::Reg::X1);
    em.branch(loop_top);
    em.finalize();

    const std::string text = em.disassemble();
    REQUIRE(text.find("b ") != std::string::npos);
    REQUIRE(text.find("sub") != std::string::npos);
}

TEST_CASE("Emitter: multiple labels are independent") {
    backend::Emitter em;
    auto a = em.create_label();
    auto b = em.create_label();
    REQUIRE(a.id != b.id);
    em.branch(a);
    em.branch(b);
    em.bind(a);
    em.movz(arm64::Reg::X0, 0xA, 0);
    em.bind(b);
    em.movz(arm64::Reg::X0, 0xB, 0);
    em.ret();
    em.finalize();
    // Finalize should not throw / assert — both labels bound.
    SUCCEED("two labels bound cleanly");
}

// ---------------------------------------------------------------------------
// F1-BK-018 literal pool management
// ---------------------------------------------------------------------------

TEST_CASE("Emitter: literal pool is reachable at start") {
    // vixl maintains a small always-present header (observed as 4 bytes)
    // in its LiteralPool even when empty, so we can't assert strict
    // zero. What matters is that the accessor is callable and returns
    // a bounded value.
    backend::Emitter em;
    REQUIRE(em.literal_pool_size() <= 16u);
}

TEST_CASE("Emitter: flush_literal_pool completes cleanly on an empty pool") {
    backend::Emitter em;
    em.movz(arm64::Reg::X0, 1, 0);
    em.flush_literal_pool();
    em.ret();
    em.finalize();
    // Finalize succeeds; disassembly still contains the surrounding code.
    REQUIRE(em.disassemble().find("ret") != std::string::npos);
}

TEST_CASE("Emitter: mov_imm64 of large immediates does NOT grow the pool") {
    // Our mov_imm64 lowers large constants via movz+movk rather than
    // ldr-literal, so the pool stays at its baseline even for values
    // with many non-zero halfwords. This test locks that invariant in.
    backend::Emitter em;
    const auto before = em.literal_pool_size();
    em.mov_imm64(arm64::Reg::X0, 0x1234'5678'9ABC'DEF0ULL);
    const auto after = em.literal_pool_size();
    REQUIRE(after == before);
    em.ret();
    em.finalize();
}
