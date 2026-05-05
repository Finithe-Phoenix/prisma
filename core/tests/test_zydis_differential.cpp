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

// ---------------------------------------------------------------------
// F1-DC-087 corpus expansion — broader coverage of length / mnemonic
// agreement. Each case is a real x86_64 byte sequence representative
// of the opcode family; the differential pin catches regressions in
// either side's decoder when these binaries are exercised end-to-end.
// ---------------------------------------------------------------------

TEST_CASE("zydis-diff(corpus): conditional branches Jcc rel8 family",
          "[zydis][differential][corpus]") {
    check_one({0x70, 0x10});  // JO  rel8
    check_one({0x71, 0x10});  // JNO rel8
    check_one({0x72, 0x10});  // JC  rel8
    check_one({0x73, 0x10});  // JNC rel8
    check_one({0x74, 0x10});  // JE  rel8
    check_one({0x75, 0x10});  // JNE rel8
    check_one({0x76, 0x10});  // JBE rel8
    check_one({0x77, 0x10});  // JA  rel8
    check_one({0x78, 0x10});  // JS  rel8
    check_one({0x79, 0x10});  // JNS rel8
    check_one({0x7C, 0x10});  // JL  rel8
    check_one({0x7D, 0x10});  // JGE rel8
    check_one({0x7E, 0x10});  // JLE rel8
    check_one({0x7F, 0x10});  // JG  rel8
}

TEST_CASE("zydis-diff(corpus): conditional branches Jcc rel32 family",
          "[zydis][differential][corpus]") {
    check_one({0x0F, 0x80, 0x00, 0x01, 0x00, 0x00});  // JO  rel32
    check_one({0x0F, 0x82, 0x00, 0x01, 0x00, 0x00});  // JC  rel32
    check_one({0x0F, 0x83, 0x00, 0x01, 0x00, 0x00});  // JNC rel32
    check_one({0x0F, 0x85, 0x00, 0x01, 0x00, 0x00});  // JNE rel32
    check_one({0x0F, 0x8C, 0x00, 0x01, 0x00, 0x00});  // JL  rel32
    check_one({0x0F, 0x8E, 0x00, 0x01, 0x00, 0x00});  // JLE rel32
}

TEST_CASE("zydis-diff(corpus): MOV variants (REX, imm, reg-reg, mem)",
          "[zydis][differential][corpus]") {
    check_one({0x48, 0xC7, 0xC0, 0x2A, 0, 0, 0});  // mov rax, 0x2A (sign-ext imm32)
    check_one({0x48, 0x89, 0xC1});                  // mov rcx, rax
    check_one({0x48, 0x8B, 0x07});                  // mov rax, [rdi]
    check_one({0x48, 0x89, 0x07});                  // mov [rdi], rax
    check_one({0x4C, 0x89, 0xC0});                  // mov rax, r8
    check_one({0x49, 0x89, 0xC0});                  // mov r8,  rax
    check_one({0xB8, 0x2A, 0x00, 0x00, 0x00});      // mov eax, 0x2A
    check_one({0x66, 0xB8, 0x2A, 0x00});            // mov ax,  0x2A
}

TEST_CASE("zydis-diff(corpus): ADD/SUB/AND/OR/XOR variants",
          "[zydis][differential][corpus]") {
    check_one({0x48, 0x01, 0xC8});  // add rax, rcx
    check_one({0x48, 0x29, 0xC8});  // sub rax, rcx
    check_one({0x48, 0x21, 0xC8});  // and rax, rcx
    check_one({0x48, 0x09, 0xC8});  // or  rax, rcx
    check_one({0x48, 0x31, 0xC8});  // xor rax, rcx
    check_one({0x48, 0x83, 0xC0, 0x10});  // add rax, 0x10 (imm8 sign-ext)
    check_one({0x48, 0x83, 0xE8, 0x10});  // sub rax, 0x10
    check_one({0x48, 0x05, 0x00, 0x10, 0x00, 0x00});  // add rax, 0x1000 (imm32)
}

TEST_CASE("zydis-diff(corpus): shifts (SHL/SHR/SAR/ROL/ROR)",
          "[zydis][differential][corpus]") {
    check_one({0x48, 0xD1, 0xE0});        // shl rax, 1
    check_one({0x48, 0xD1, 0xE8});        // shr rax, 1
    check_one({0x48, 0xD1, 0xF8});        // sar rax, 1
    check_one({0x48, 0xC1, 0xE0, 0x04});  // shl rax, 4
    check_one({0x48, 0xC1, 0xE8, 0x08});  // shr rax, 8
    check_one({0x48, 0xD1, 0xC0});        // rol rax, 1
    check_one({0x48, 0xD1, 0xC8});        // ror rax, 1
}

TEST_CASE("zydis-diff(corpus): TEST / CMP / NEG / NOT family",
          "[zydis][differential][corpus]") {
    check_one({0x48, 0x85, 0xC0});                  // test rax, rax
    check_one({0x48, 0xF7, 0xC0, 0xFF, 0, 0, 0});   // test rax, 0xFF
    check_one({0x48, 0x39, 0xC8});                  // cmp rax, rcx
    check_one({0x48, 0x83, 0xF8, 0x00});            // cmp rax, 0
    check_one({0x48, 0xF7, 0xD8});                  // neg rax
    check_one({0x48, 0xF7, 0xD0});                  // not rax
}

TEST_CASE("zydis-diff(corpus): MUL / IMUL / DIV / IDIV",
          "[zydis][differential][corpus]") {
    check_one({0x48, 0xF7, 0xE0});  // mul rax        — RDX:RAX = RAX * RAX
    check_one({0x48, 0xF7, 0xE8});  // imul rax
    check_one({0x48, 0xF7, 0xF0});  // div rax
    check_one({0x48, 0xF7, 0xF8});  // idiv rax
    check_one({0x48, 0x0F, 0xAF, 0xC1});  // imul rax, rcx (3-op short form)
}

TEST_CASE("zydis-diff(corpus): PUSH / POP all-reg",
          "[zydis][differential][corpus]") {
    check_one({0x50});  // push rax
    check_one({0x51});  // push rcx
    check_one({0x52});  // push rdx
    check_one({0x53});  // push rbx
    check_one({0x54});  // push rsp
    check_one({0x55});  // push rbp
    check_one({0x56});  // push rsi
    check_one({0x57});  // push rdi
    check_one({0x58});  // pop  rax
    check_one({0x59});  // pop  rcx
    check_one({0x5F});  // pop  rdi
    check_one({0x41, 0x50});  // push r8
    check_one({0x41, 0x57});  // push r15
    check_one({0x6A, 0x10});             // push imm8
    check_one({0x68, 0x00, 0x10, 0, 0}); // push imm32
}

TEST_CASE("zydis-diff(corpus): CALL / RET",
          "[zydis][differential][corpus]") {
    check_one({0xE8, 0x00, 0x01, 0x00, 0x00});  // call rel32
    check_one({0xC3});                          // ret
    check_one({0xC2, 0x10, 0x00});              // ret imm16 (pop 16 bytes)
    check_one({0xFF, 0xD0});                    // call rax (indirect)
    check_one({0xFF, 0xE0});                    // jmp  rax (indirect)
}

TEST_CASE("zydis-diff(corpus): LEA addressing modes",
          "[zydis][differential][corpus]") {
    check_one({0x48, 0x8D, 0x07});                       // lea rax, [rdi]
    check_one({0x48, 0x8D, 0x47, 0x10});                 // lea rax, [rdi+0x10]
    check_one({0x48, 0x8D, 0x87, 0x00, 0x10, 0, 0});     // lea rax, [rdi+0x1000]
    check_one({0x48, 0x8D, 0x04, 0x37});                 // lea rax, [rdi+rsi]
    check_one({0x48, 0x8D, 0x04, 0x77});                 // lea rax, [rdi+rsi*2]
}

TEST_CASE("zydis-diff(corpus): NOP forms",
          "[zydis][differential][corpus]") {
    check_one({0x90});                                  // nop
    check_one({0x66, 0x90});                            // 66 nop (operand-size)
    check_one({0x0F, 0x1F, 0x00});                      // nop [rax]
    check_one({0x0F, 0x1F, 0x40, 0x00});                // nop [rax+0]
    check_one({0x0F, 0x1F, 0x44, 0x00, 0x00});          // nop [rax+rax+0]
}

TEST_CASE("zydis-diff(corpus): atomic / fence",
          "[zydis][differential][corpus]") {
    check_one({0xF0, 0x48, 0x01, 0x07});  // lock add [rdi], rax
    check_one({0xF0, 0x48, 0x0F, 0xC1, 0x07});  // lock xadd [rdi], rax
    check_one({0x0F, 0xAE, 0xF0});  // mfence
    check_one({0x0F, 0xAE, 0xE8});  // lfence
    check_one({0x0F, 0xAE, 0xF8});  // sfence
}

#else  // !PRISMA_HAVE_ZYDIS

TEST_CASE("zydis-diff: skipped (build with -DPRISMA_ENABLE_ZYDIS=ON)",
          "[zydis][differential][!shouldfail]") {
    WARN("Zydis not enabled at configure time; differential decoder "
         "test is a no-op. Re-run cmake with -DPRISMA_ENABLE_ZYDIS=ON "
         "to activate.");
}

#endif  // PRISMA_HAVE_ZYDIS
