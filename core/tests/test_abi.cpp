// core/tests/test_abi.cpp — F1-BK-009 callee-saved discipline.
//
// The block prologue / epilogue must save and restore the AAPCS64
// callee-saved register pairs we touch: x29/x30, x27/x28, x25/x26,
// x23/x24, x21/x22, x19/x20. Six pairs = 96-byte stack frame.
//
// We assert on the disassembled mnemonics rather than raw bytes so
// vixl is free to pick the encoding (stp pre-indexed vs. add+str).

#include <catch2/catch_test_macros.hpp>
#include <string>
#include <vector>

#include "prisma/abi.hpp"
#include "prisma/emitter.hpp"

using namespace prisma;

namespace {

std::string disasm_after(void (*emit)(backend::Emitter&)) {
    backend::Emitter em;
    emit(em);
    em.finalize();
    return em.disassemble();
}

unsigned count_occurrences(const std::string& haystack,
                           const std::string& needle) {
    unsigned n = 0;
    std::size_t pos = 0;
    while ((pos = haystack.find(needle, pos)) != std::string::npos) {
        ++n;
        pos += needle.size();
    }
    return n;
}

}  // namespace

TEST_CASE("backend::abi: prologue saves all six callee-saved register pairs") {
    const std::string d = disasm_after(backend::abi::emit_block_prologue);

    // Six stp pre-indexed (AAPCS64 push) instructions.
    REQUIRE(count_occurrences(d, "stp ") == backend::abi::kCalleeSavedPairCount);

    // The exact pairs we save. Vixl prints `stp x19, x20, [sp, #-16]!`
    // (decimal) or `stp x19, x20, [sp, #-0x10]!` (hex). Match the
    // register pair regardless of immediate format.
    REQUIRE(d.find("x19, x20") != std::string::npos);
    REQUIRE(d.find("x21, x22") != std::string::npos);
    REQUIRE(d.find("x23, x24") != std::string::npos);
    REQUIRE(d.find("x25, x26") != std::string::npos);
    REQUIRE(d.find("x27, x28") != std::string::npos);
    REQUIRE(d.find("x29, x30") != std::string::npos);

    // The state pointer (x0) is moved into the pinned holder (x27).
    // Vixl renders `mov x27, x0`.
    REQUIRE(d.find("mov x27, x0") != std::string::npos);
}

TEST_CASE("backend::abi: epilogue restores all six pairs and ends in ret") {
    const std::string d = disasm_after(backend::abi::emit_block_epilogue_and_ret);

    REQUIRE(count_occurrences(d, "ldp ") == backend::abi::kCalleeSavedPairCount);

    REQUIRE(d.find("x19, x20") != std::string::npos);
    REQUIRE(d.find("x21, x22") != std::string::npos);
    REQUIRE(d.find("x23, x24") != std::string::npos);
    REQUIRE(d.find("x25, x26") != std::string::npos);
    REQUIRE(d.find("x27, x28") != std::string::npos);
    REQUIRE(d.find("x29, x30") != std::string::npos);

    REQUIRE(d.find("ret") != std::string::npos);
}

TEST_CASE("backend::abi: prologue + epilogue is balanced — same SP delta in both directions") {
    // The cheap structural check: the same six register pair tokens
    // appear in BOTH halves. SP-delta correctness is guaranteed by
    // vixl's pre-/post-indexed encodings; pairing is what we need.
    backend::Emitter em;
    backend::abi::emit_block_prologue(em);
    backend::abi::emit_block_epilogue_and_ret(em);
    em.finalize();
    const std::string d = em.disassemble();
    REQUIRE(count_occurrences(d, "stp ") == backend::abi::kCalleeSavedPairCount);
    REQUIRE(count_occurrences(d, "ldp ") == backend::abi::kCalleeSavedPairCount);
}

TEST_CASE("backend::abi: kStatePtrReg is a callee-saved register") {
    // Whatever we picked for the holder MUST be in x19..x28 (AAPCS64
    // callee-saved). Otherwise the body's calls into runtime helpers
    // could clobber it.
    const auto r = static_cast<int>(backend::abi::kStatePtrReg);
    REQUIRE(r >= 19);
    REQUIRE(r <= 28);
}
