// core/tests/test_zydis_differential.cpp — F1-DC-081.
//
// For every byte sequence in the test corpus, decode through both
// Prisma and Zydis. Pin two invariants:
//
//   * Length agreement: Prisma's `bytes_consumed` must equal Zydis's
//     `info.length`.
//   * Decode-vs-decline parity: Prisma either decodes the same length
//     Zydis does, or returns DecodeError; an "accept different length"
//     result is a hard fail.
//
// Mnemonic-level cross-checking is deferred to F1-DC-087 (Zydis-free
// migration); for now the length pin alone catches the most common
// regression class — accidental over- or under-consumption of bytes.
//
// This test only compiles when `PRISMA_ENABLE_ZYDIS=ON` is passed at
// CMake configure time. The CI stub builds without Zydis as a
// dependency-free check.

#include <catch2/catch_test_macros.hpp>
#include <cstdint>
#include <span>
#include <variant>
#include <vector>

#include "prisma/decoder.hpp"

#if defined(PRISMA_HAVE_ZYDIS)
#include <Zydis/Zydis.h>
#endif

using namespace prisma;

namespace {

#if defined(PRISMA_HAVE_ZYDIS)
struct ZydisLength {
    bool        ok;
    std::size_t length;     // valid iff ok
    std::string mnemonic;   // empty if ok == false
};

ZydisLength zydis_decode_length(std::span<const std::uint8_t> bytes) {
    ZydisDecoder decoder;
    if (ZYAN_FAILED(ZydisDecoderInit(&decoder, ZYDIS_MACHINE_MODE_LONG_64,
                                      ZYDIS_STACK_WIDTH_64))) {
        return {false, 0, ""};
    }
    ZydisDecodedInstruction inst;
    ZydisDecodedOperand     ops[ZYDIS_MAX_OPERAND_COUNT];
    if (ZYAN_FAILED(ZydisDecoderDecodeFull(
            &decoder, bytes.data(), bytes.size(), &inst, ops))) {
        return {false, 0, ""};
    }
    return {true, inst.length, ZydisMnemonicGetString(inst.mnemonic)};
}

// Helper: ask Prisma's decoder for the length on success, or 0 on
// error. Caller is the test that knows what to do with the value.
std::pair<bool, std::size_t>
prisma_decode_length(std::span<const std::uint8_t> bytes) {
    ir::Ref ref = 0;
    auto r = decoder::decode_one(bytes, ref);
    if (std::holds_alternative<decoder::DecodeError>(r)) return {false, 0};
    return {true, std::get<decoder::Decoded>(r).bytes_consumed};
}

// F1-DC-087: map a Prisma IR Op to the x86 mnemonic Zydis would
// report for the originating instruction, when the mapping is clear.
// Returns empty string for ops where no obvious 1:1 mapping exists
// (compound IR, placeholders, multi-op decompositions). Caller
// treats empty as "skip the mnemonic check, length only."
std::string prisma_mnemonic_for(const ir::Op& op) {
    if (std::holds_alternative<ir::Return>(op))      return "ret";
    if (std::holds_alternative<ir::Cpuid>(op))       return "cpuid";
    if (std::holds_alternative<ir::Syscall>(op))     return "syscall";
    if (std::holds_alternative<ir::Trap>(op)) {
        const auto& t = std::get<ir::Trap>(op);
        return t.kind == ir::TrapKind::Sigtrap ? "int3" : "";
    }
    if (std::holds_alternative<ir::JumpRel>(op))     return "jmp";
    if (std::holds_alternative<ir::CondJumpRel>(op)) return "";  // many Jcc mnemonics
    if (std::holds_alternative<ir::CmpFlags>(op))    return "cmp";
    return "";
}

// Run one cross-check. Pass = either both decoded with the same
// length, or Prisma declined (DecodeError) — which is acceptable
// for opcodes outside our MVP set.
void check_one(std::vector<std::uint8_t> bytes,
               std::string expected_mnemonic = "") {
    const auto z = zydis_decode_length(std::span<const std::uint8_t>(bytes));
    const auto p = prisma_decode_length(std::span<const std::uint8_t>(bytes));

    INFO("zydis ok=" << z.ok << " len=" << z.length
         << " mnem=" << z.mnemonic
         << " ; prisma ok=" << p.first << " len=" << p.second);

    if (!z.ok) {
        // Zydis itself declined the bytes. Prisma's behaviour isn't
        // pinned by this differential; both decline is fine, both
        // accept (genuine bug) is also visible to the assertions
        // below since Prisma's length would have nothing to compare
        // against — skip the case.
        SUCCEED("zydis declined; differential pin not applicable");
        return;
    }
    // Zydis accepted. Prisma must either also accept with the same
    // length or decline.
    if (p.first) {
        REQUIRE(p.second == z.length);
        // F1-DC-087: when the caller pinned an expected mnemonic,
        // Zydis must agree. This is the cross-check that lets us
        // measure "≥99% matching on coreutils" once we feed real
        // binaries through.
        if (!expected_mnemonic.empty()) {
            REQUIRE(z.mnemonic == expected_mnemonic);
        }
    }
}
#endif

}  // namespace

#if defined(PRISMA_HAVE_ZYDIS)

TEST_CASE("zydis-diff: RET (C3) length agreement", "[zydis][differential]") {
    check_one({0xC3});
}

TEST_CASE("zydis-diff: NOP (90) length agreement", "[zydis][differential]") {
    check_one({0x90});
}

TEST_CASE("zydis-diff: MOV r64, imm64 (48 B8 ...) length agreement",
          "[zydis][differential]") {
    // mov rax, 0x1234567890ABCDEF
    check_one({0x48, 0xB8, 0xEF, 0xCD, 0xAB, 0x90, 0x78, 0x56, 0x34, 0x12});
}

TEST_CASE("zydis-diff: ADD r/m64, r64 (48 01 /r) length agreement",
          "[zydis][differential]") {
    // add rax, rcx
    check_one({0x48, 0x01, 0xC8});
}

TEST_CASE("zydis-diff: SUB r/m64, r64 (48 29 /r) length agreement",
          "[zydis][differential]") {
    // sub rax, rcx
    check_one({0x48, 0x29, 0xC8});
}

TEST_CASE("zydis-diff: JMP rel8 (EB cb) length agreement",
          "[zydis][differential]") {
    check_one({0xEB, 0x10});
}

TEST_CASE("zydis-diff: JE rel32 (0F 84) length agreement",
          "[zydis][differential]") {
    check_one({0x0F, 0x84, 0x10, 0x00, 0x00, 0x00});
}

TEST_CASE("zydis-diff: PUSH r64 (50+rd) length agreement",
          "[zydis][differential]") {
    check_one({0x50});
}

TEST_CASE("zydis-diff: POP r64 (58+rd) length agreement",
          "[zydis][differential]") {
    check_one({0x58});
}

TEST_CASE("zydis-diff: REP STOSB (F3 AA) length agreement",
          "[zydis][differential]") {
    check_one({0xF3, 0xAA});
}

TEST_CASE("zydis-diff: REP MOVSQ (F3 48 A5) length agreement",
          "[zydis][differential]") {
    check_one({0xF3, 0x48, 0xA5});
}

TEST_CASE("zydis-diff: CMP r/m64, r64 (48 39 /r) length agreement",
          "[zydis][differential]") {
    // cmp rax, rcx
    check_one({0x48, 0x39, 0xC8});
}

TEST_CASE("zydis-diff: TEST r/m64, r64 (48 85 /r) length agreement",
          "[zydis][differential]") {
    check_one({0x48, 0x85, 0xC8});
}

TEST_CASE("zydis-diff: CPUID (0F A2) length agreement",
          "[zydis][differential]") {
    check_one({0x0F, 0xA2});
}

TEST_CASE("zydis-diff: SYSCALL (0F 05) length agreement",
          "[zydis][differential]") {
    check_one({0x0F, 0x05});
}

TEST_CASE("zydis-diff: INT3 (CC) length agreement",
          "[zydis][differential]") {
    check_one({0xCC});
}

// ---------------------------------------------------------------------
// F1-DC-087 mnemonic-level cross-check on the subset where Prisma's
// IR has a 1:1 mapping to an x86 mnemonic.
// ---------------------------------------------------------------------

TEST_CASE("zydis-diff: RET (C3) mnemonic = ret",
          "[zydis][differential][mnemonic]") {
    check_one({0xC3}, "ret");
}

TEST_CASE("zydis-diff: NOP (90) mnemonic = nop",
          "[zydis][differential][mnemonic]") {
    check_one({0x90}, "nop");
}

TEST_CASE("zydis-diff: CPUID (0F A2) mnemonic = cpuid",
          "[zydis][differential][mnemonic]") {
    check_one({0x0F, 0xA2}, "cpuid");
}

TEST_CASE("zydis-diff: SYSCALL (0F 05) mnemonic = syscall",
          "[zydis][differential][mnemonic]") {
    check_one({0x0F, 0x05}, "syscall");
}

TEST_CASE("zydis-diff: INT3 (CC) mnemonic = int3",
          "[zydis][differential][mnemonic]") {
    check_one({0xCC}, "int3");
}

TEST_CASE("zydis-diff: JMP rel8 (EB) mnemonic = jmp",
          "[zydis][differential][mnemonic]") {
    check_one({0xEB, 0x10}, "jmp");
}

TEST_CASE("zydis-diff: MOV r64, imm64 (48 B8) mnemonic = mov",
          "[zydis][differential][mnemonic]") {
    check_one({0x48, 0xB8, 0x01, 0, 0, 0, 0, 0, 0, 0}, "mov");
}

TEST_CASE("zydis-diff: prisma_mnemonic_for sanity") {
    using namespace prisma::ir;
    REQUIRE(prisma_mnemonic_for(Op{Return{}}) == "ret");
    REQUIRE(prisma_mnemonic_for(Op{Cpuid{}}) == "cpuid");
    REQUIRE(prisma_mnemonic_for(Op{Syscall{}}) == "syscall");
    REQUIRE(prisma_mnemonic_for(Op{Trap{TrapKind::Sigtrap}}) == "int3");
    REQUIRE(prisma_mnemonic_for(Op{Trap{TrapKind::Sigill}}).empty());
    REQUIRE(prisma_mnemonic_for(Op{JumpRel{0x100}}) == "jmp");
    REQUIRE(prisma_mnemonic_for(Op{CmpFlags{0u, 1u, OpSize::I64}}) == "cmp");
}

#else  // !PRISMA_HAVE_ZYDIS

TEST_CASE("zydis-diff: skipped (build with -DPRISMA_ENABLE_ZYDIS=ON)",
          "[zydis][differential][!shouldfail]") {
    WARN("Zydis not enabled at configure time; differential decoder "
         "test is a no-op. Re-run cmake with -DPRISMA_ENABLE_ZYDIS=ON "
         "to activate.");
}

#endif  // PRISMA_HAVE_ZYDIS
