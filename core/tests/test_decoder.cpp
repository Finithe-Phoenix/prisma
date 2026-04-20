// core/tests/test_decoder.cpp — tests for the MVP x86_64 decoder.
//
// Each test includes the raw bytes of a real x86_64 instruction (verified
// against Intel SDM / objdump) and asserts the IR produced. For compound
// instructions the result order matches the decoder's emission order.

#include <catch2/catch_test_macros.hpp>

#include "prisma/decoder.hpp"
#include "prisma/ir.hpp"

using namespace prisma;
using prisma::decoder::decode_one;
using prisma::decoder::DecodeError;
using prisma::decoder::Decoded;

namespace {

// Small helper: decode one instruction, require success, return the result.
// `std::span` does not implicitly construct from an initializer_list, so we
// explicitly take a vector here and convert.
Decoded decode_ok(std::vector<std::uint8_t> bytes, ir::Ref& ref) {
    auto r = decode_one(std::span<const std::uint8_t>{bytes}, ref);
    REQUIRE(std::holds_alternative<Decoded>(r));
    return std::get<Decoded>(r);
}

// Same idea for tests that expect a DecodeError.
auto decode_any(std::vector<std::uint8_t> bytes, ir::Ref& ref) {
    return decode_one(std::span<const std::uint8_t>{bytes}, ref);
}

}  // namespace

TEST_CASE("decode RET (C3) → Return, 1 byte") {
    ir::Ref r = 0;
    auto d = decode_ok({0xC3}, r);
    REQUIRE(d.bytes_consumed == 1);
    REQUIRE(d.stmts.size() == 1);
    REQUIRE(d.stmts[0].result == std::nullopt);
    REQUIRE(d.stmts[0].op == ir::Op{ir::Return{}});
    REQUIRE(r == 0);  // RET does not produce SSA values
}

TEST_CASE("decode MOV rax, 0x0123456789ABCDEF → Constant + StoreReg, 10 bytes") {
    // Encoding: 48 B8 EF CD AB 89 67 45 23 01
    //           ^^ REX.W
    //              ^^ opcode B8+0 (rax)
    //                 ^^ imm64 little-endian
    ir::Ref r = 0;
    auto d = decode_ok(
        {0x48, 0xB8, 0xEF, 0xCD, 0xAB, 0x89, 0x67, 0x45, 0x23, 0x01}, r);
    REQUIRE(d.bytes_consumed == 10);
    REQUIRE(d.stmts.size() == 2);

    // %0 = const.i64 0x0123456789ABCDEF
    REQUIRE(d.stmts[0].result == std::optional<ir::Ref>(0));
    REQUIRE(d.stmts[0].op == ir::Op{ir::Constant{0x0123'4567'89AB'CDEFULL, ir::OpSize::I64}});

    // storereg.i64 rax, %0
    REQUIRE(d.stmts[1].result == std::nullopt);
    REQUIRE(d.stmts[1].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 0u, ir::OpSize::I64}});

    REQUIRE(r == 1);
}

TEST_CASE("decode MOV rcx, 0 (the tidiest possible 64-bit mov)") {
    ir::Ref r = 0;
    auto d = decode_ok(
        {0x48, 0xB9, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00}, r);
    REQUIRE(d.bytes_consumed == 10);
    REQUIRE(d.stmts.size() == 2);
    REQUIRE(d.stmts[0].op == ir::Op{ir::Constant{0, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rcx, 0u, ir::OpSize::I64}});
}

TEST_CASE("decode ADD rax, rbx → LoadReg+LoadReg+BinOp+StoreReg, 3 bytes") {
    // Encoding: 48 01 D8
    //   48 REX.W
    //   01 opcode ADD r/m64, r64
    //   D8 mod=11, reg=011 (rbx, source), rm=000 (rax, destination)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x01, 0xD8}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 4);

    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});

    REQUIRE(r == 3);
}

TEST_CASE("decode SUB rdx, rcx → same shape, BinOpKind::Sub, 3 bytes") {
    // Encoding: 48 29 CA
    //   48 REX.W
    //   29 opcode SUB r/m64, r64
    //   CA mod=11, reg=001 (rcx, source), rm=010 (rdx, destination)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x29, 0xCA}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rdx, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rcx, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Sub, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rdx, 2u, ir::OpSize::I64}});
}

TEST_CASE("next_ref is threaded across multiple decodes") {
    // Decode two instructions in sequence: ADD rax, rbx ; RET.
    // The second decode must NOT reset refs.
    ir::Ref r = 0;
    auto add = decode_ok({0x48, 0x01, 0xD8}, r);
    REQUIRE(r == 3);
    auto ret = decode_ok({0xC3}, r);
    REQUIRE(r == 3);  // RET adds no refs
    REQUIRE(ret.stmts.size() == 1);
    REQUIRE(ret.stmts[0].op == ir::Op{ir::Return{}});
}

TEST_CASE("Error: empty input → TruncatedInput") {
    ir::Ref r = 0;
    auto res = decode_any({}, r);
    REQUIRE(std::holds_alternative<DecodeError>(res));
    REQUIRE(std::get<DecodeError>(res) == DecodeError::TruncatedInput);
}

TEST_CASE("Error: MOV r64 imm64 truncated mid-immediate") {
    ir::Ref r = 0;
    // Only 4 bytes of immediate instead of 8.
    auto res = decode_any({0x48, 0xB8, 0x01, 0x02, 0x03, 0x04}, r);
    REQUIRE(std::holds_alternative<DecodeError>(res));
    REQUIRE(std::get<DecodeError>(res) == DecodeError::TruncatedInput);
}

TEST_CASE("Error: unknown opcode") {
    ir::Ref r = 0;
    auto res = decode_any({0xFF}, r);
    REQUIRE(std::holds_alternative<DecodeError>(res));
    REQUIRE(std::get<DecodeError>(res) == DecodeError::UnknownOpcode);
}

TEST_CASE("Error: ADD with memory operand rejected as UnsupportedEncoding") {
    // 48 01 18  →  ADD [rax], rbx  (mod=00, rm=000 — memory indirect).
    ir::Ref r = 0;
    auto res = decode_any({0x48, 0x01, 0x18}, r);
    REQUIRE(std::holds_alternative<DecodeError>(res));
    REQUIRE(std::get<DecodeError>(res) == DecodeError::UnsupportedEncoding);
}

TEST_CASE("Error: REX.R rejected for MVP (would reference R8..R15)") {
    // 4C 01 D8  →  ADD rax, r11  (REX.R=1 extends reg field).
    ir::Ref r = 0;
    auto res = decode_any({0x4C, 0x01, 0xD8}, r);
    REQUIRE(std::holds_alternative<DecodeError>(res));
    REQUIRE(std::get<DecodeError>(res) == DecodeError::UnsupportedEncoding);
}

// ---------------------------------------------------------------------------
// G1: additional register-register ops and NOP.
// ---------------------------------------------------------------------------

TEST_CASE("decode NOP (90) → empty stmts, 1 byte consumed") {
    ir::Ref r = 0;
    auto d = decode_ok({0x90}, r);
    REQUIRE(d.bytes_consumed == 1);
    REQUIRE(d.stmts.empty());
    REQUIRE(r == 0);  // no SSA values produced
}

TEST_CASE("decode MOV rax, rbx → loadreg + storereg, 3 bytes") {
    // 48 89 D8
    //   48 REX.W
    //   89 opcode MOV r/m64, r64
    //   D8 mod=11, reg=011 (rbx, source), rm=000 (rax, destination)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x89, 0xD8}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 2);

    // %0 = loadreg.i64 rbx
    REQUIRE(d.stmts[0].result == std::optional<ir::Ref>(0));
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    // storereg.i64 rax, %0
    REQUIRE(d.stmts[1].result == std::nullopt);
    REQUIRE(d.stmts[1].op == ir::Op{ir::StoreReg{ir::Gpr::Rax, 0u, ir::OpSize::I64}});

    REQUIRE(r == 1);
}

TEST_CASE("decode XOR rax, rax → the zero idiom") {
    // 48 31 C0
    //   31 XOR r/m64, r64
    //   C0 mod=11, reg=000 (rax), rm=000 (rax)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x31, 0xC0}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Xor, 0u, 1u, ir::OpSize::I64}});
    // Later: a simple constant-folding pass will turn XOR(x, x) into a zero
    // constant + StoreReg. Tracked as a test fixture for passes/const_prop.
}

TEST_CASE("decode AND rsi, rdi → BinOpKind::And") {
    // 48 21 FE
    //   21 AND r/m64, r64
    //   FE mod=11, reg=111 (rdi), rm=110 (rsi)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x21, 0xFE}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rsi, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rdi, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::And, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rsi, 2u, ir::OpSize::I64}});
}

TEST_CASE("decode OR rbp, rsp → BinOpKind::Or") {
    // 48 09 E5
    //   09 OR r/m64, r64
    //   E5 mod=11, reg=100 (rsp), rm=101 (rbp)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x09, 0xE5}, r);
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Or, 0u, 1u, ir::OpSize::I64}});
}

// ---------------------------------------------------------------------------
// H3: memory operands.
// ---------------------------------------------------------------------------

TEST_CASE("decode MOV [rbx], rax → LoadReg+LoadReg+StoreMemTSO") {
    // 48 89 03
    //   89 MOV r/m64, r64
    //   03 mod=00, reg=000 (rax, src), rm=011 (rbx → [rbx])
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x89, 0x03}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 3);

    // %0 = loadreg rax (the source value)
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    // %1 = loadreg rbx (the base address)
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    // store.tso.i64 [%1], %0
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::StoreMemTSO{1u, 0u, ir::OpSize::I64}});
}

TEST_CASE("decode MOV rcx, [rdx + 0x10] → disp8 memory form") {
    // 48 8B 4A 10
    //   8B MOV r64, r/m64
    //   4A mod=01, reg=001 (rcx, dst), rm=010 (rdx)
    //   10 disp8 = +16
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x8B, 0x4A, 0x10}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 5);

    // %0 = loadreg rdx (base)
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rdx, ir::OpSize::I64}});
    // %1 = const 0x10
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{0x10, ir::OpSize::I64}});
    // %2 = add %0, %1
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}});
    // %3 = load.tso.i64 [%2]
    REQUIRE(d.stmts[3].op == ir::Op{ir::LoadMemTSO{2u, ir::OpSize::I64}});
    // storereg rcx, %3
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rcx, 3u, ir::OpSize::I64}});
}

// ---------------------------------------------------------------------------
// H4: additional MOV widths (I8/I16/I32) and MOV r8/imm8.
// ---------------------------------------------------------------------------

TEST_CASE("decode MOV eax, 0x12345678 → Constant + StoreReg, 5 bytes") {
    ir::Ref r = 0;
    auto d = decode_ok({0xB8, 0x78, 0x56, 0x34, 0x12}, r);
    REQUIRE(d.bytes_consumed == 5);
    REQUIRE(d.stmts.size() == 2);
    REQUIRE(d.stmts[0].op == ir::Op{ir::Constant{0x1234'5678ULL, ir::OpSize::I32}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::StoreReg{ir::Gpr::Rax, 0u, ir::OpSize::I32}});
}

TEST_CASE("decode MOV al, 0xCC → Constant + StoreReg, 2 bytes") {
    ir::Ref r = 0;
    auto d = decode_ok({0xB0, 0xCC}, r);
    REQUIRE(d.bytes_consumed == 2);
    REQUIRE(d.stmts.size() == 2);
    REQUIRE(d.stmts[0].op == ir::Op{ir::Constant{0xCCULL, ir::OpSize::I8}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::StoreReg{ir::Gpr::Rax, 0u, ir::OpSize::I8}});
}

TEST_CASE("decode MOV rcx, rbx via 0x89 without REX.W is I32") {
    // 89 D8 (MOV r/m32, r32): reg=011 (rbx), rm=000 (rax / eax)
    ir::Ref r = 0;
    auto d = decode_ok({0x89, 0xD8}, r);
    REQUIRE(d.bytes_consumed == 2);
    REQUIRE(d.stmts.size() == 2);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I32}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::StoreReg{ir::Gpr::Rax, 0u, ir::OpSize::I32}});
}

TEST_CASE("decode MOV [rbx], eax for I32 memory form (89 /r)") {
    ir::Ref r = 0;
    auto d = decode_ok({0x89, 0x03}, r);
    REQUIRE(d.bytes_consumed == 2);
    REQUIRE(d.stmts.size() == 3);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I32}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::StoreMemTSO{1u, 0u, ir::OpSize::I32}});
}

TEST_CASE("decode MOV [rbx], ax with 0x66 prefix (66 89 /r), I16 register direct") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x89, 0xD8}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 2);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I16}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::StoreReg{ir::Gpr::Rax, 0u, ir::OpSize::I16}});
}

TEST_CASE("decode MOV [rbx], ax with 0x66 prefix (66 89 /r), I16 memory form") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x89, 0x03}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 3);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I16}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].result == std::optional<ir::Ref>(1u));
    REQUIRE(d.stmts[2].op == ir::Op{ir::StoreMemTSO{1u, 0u, ir::OpSize::I16}});
}

TEST_CASE("decode MOV [rbx], al with 88 /r (I8 register direct)") {
    ir::Ref r = 0;
    auto d = decode_ok({0x88, 0xC1}, r);
    REQUIRE(d.bytes_consumed == 2);
    REQUIRE(d.stmts.size() == 2);
    // 88 C1: mov cl, al
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I8}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::StoreReg{ir::Gpr::Rcx, 0u, ir::OpSize::I8}});
}

TEST_CASE("decode MOV [rbx], al with 88 /r (I8 memory form)") {
    ir::Ref r = 0;
    auto d = decode_ok({0x88, 0x03}, r);
    REQUIRE(d.bytes_consumed == 2);
    REQUIRE(d.stmts.size() == 3);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I8}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::StoreMemTSO{1u, 0u, ir::OpSize::I8}});
}

TEST_CASE("decode MOV rdi, [rsi] → no-disp memory load") {
    // 48 8B 3E
    //   3E mod=00, reg=111 (rdi), rm=110 (rsi)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x8B, 0x3E}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 3);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rsi, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadMemTSO{0u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rdi, 1u, ir::OpSize::I64}});
}

TEST_CASE("decode MOV [rbx - 8], rax → disp8 negative via sign extension") {
    // 48 89 43 F8
    //   43 mod=01, reg=000 (rax), rm=011 (rbx)
    //   F8 disp8 = -8 (sign-extended)
    //
    // Emission order: loadreg(rax) → loadreg(rbx) → const(-8) → add → store.
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x89, 0x43, 0xF8}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 5);

    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    // The constant the address computation uses must be -8 sign-extended to
    // 64-bit: 0xFFFF_FFFF_FFFF_FFF8.
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::Constant{0xFFFF'FFFF'FFFF'FFF8ULL, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Add, 1u, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::StoreMemTSO{3u, 0u, ir::OpSize::I64}});
}

TEST_CASE("decode MOV rax, [rbx + 0x0BADF00D] → disp32 memory form") {
    // 48 8B 83 0D F0 AD 0B
    //   83 mod=10, reg=000 (rax), rm=011 (rbx)
    //   0D F0 AD 0B → disp32 = 0x0BADF00D little-endian
    ir::Ref r = 0;
    auto d = decode_ok(
        {0x48, 0x8B, 0x83, 0x0D, 0xF0, 0xAD, 0x0B}, r);
    REQUIRE(d.bytes_consumed == 7);
    REQUIRE(d.stmts.size() == 5);
    REQUIRE(d.stmts[1].op ==
            ir::Op{ir::Constant{0x0BAD'F00DULL, ir::OpSize::I64}});
}

TEST_CASE("decode MOV rax, rbx via 0x8B (register direct)") {
    // 48 8B C3
    //   C3 mod=11, reg=000 (rax, dst), rm=011 (rbx, src)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x8B, 0xC3}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 2);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 0u, ir::OpSize::I64}});
}

TEST_CASE("Error: memory form MOV with SIB byte (rm=100) is rejected") {
    // 48 89 04 ... would require SIB. We stop at the ModR/M.
    ir::Ref r = 0;
    auto res = decode_any({0x48, 0x89, 0x04}, r);
    REQUIRE(std::holds_alternative<DecodeError>(res));
    REQUIRE(std::get<DecodeError>(res) == DecodeError::UnsupportedEncoding);
}

TEST_CASE("Error: mod=00 rm=101 (disp32 absolute) is rejected") {
    // 48 89 05 XX XX XX XX  (MOV [rip+disp32], rax). RIP-relative addressing
    // is the x86-64 re-use of rm=101. Needs CFG awareness to map; out of scope.
    ir::Ref r = 0;
    auto res = decode_any({0x48, 0x89, 0x05, 0x00, 0x00, 0x00, 0x00}, r);
    REQUIRE(std::holds_alternative<DecodeError>(res));
    REQUIRE(std::get<DecodeError>(res) == DecodeError::UnsupportedEncoding);
}

TEST_CASE("Error: truncated disp8 returns TruncatedInput") {
    // 48 89 43  ← missing the disp8 byte
    ir::Ref r = 0;
    auto res = decode_any({0x48, 0x89, 0x43}, r);
    REQUIRE(std::holds_alternative<DecodeError>(res));
    REQUIRE(std::get<DecodeError>(res) == DecodeError::TruncatedInput);
}

TEST_CASE("Error: truncated disp32 returns TruncatedInput") {
    ir::Ref r = 0;
    auto res = decode_any({0x48, 0x8B, 0x83, 0x01, 0x02}, r);
    REQUIRE(std::holds_alternative<DecodeError>(res));
    REQUIRE(std::get<DecodeError>(res) == DecodeError::TruncatedInput);
}

// ---------------------------------------------------------------------------
// Control-flow opcodes.
// ---------------------------------------------------------------------------

TEST_CASE("decode CMP rax, rbx → LoadReg + LoadReg + CmpFlags") {
    // 48 39 D8
    //   48 REX.W
    //   39 opcode CMP r/m64, r64
    //   D8 mod=11, reg=011 (rbx, src), rm=000 (rax, dst)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x39, 0xD8}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 3);

    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::CmpFlags{0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].result == std::nullopt);  // side-effecting
}

TEST_CASE("decode JMP rel8 computes absolute target from instruction_guest_pc") {
    // EB 05 → jmp +5 from end-of-instruction
    // Starting PC = 0x1000, length 2, target = 0x1000 + 2 + 5 = 0x1007.
    ir::Ref r = 0;
    auto res = decoder::decode_one(
        std::span<const std::uint8_t>{std::vector<std::uint8_t>{0xEB, 0x05}},
        r, /*instruction_guest_pc=*/0x1000);
    REQUIRE(std::holds_alternative<Decoded>(res));
    const auto& d = std::get<Decoded>(res);
    REQUIRE(d.bytes_consumed == 2);
    REQUIRE(d.stmts.size() == 1);
    REQUIRE(d.stmts[0].op == ir::Op{ir::JumpRel{0x1007}});
}

TEST_CASE("decode JMP rel8 with negative displacement wraps via sign extension") {
    // EB FE → jmp -2 = infinite loop back to this instruction.
    ir::Ref r = 0;
    auto res = decoder::decode_one(
        std::span<const std::uint8_t>{std::vector<std::uint8_t>{0xEB, 0xFE}},
        r, /*instruction_guest_pc=*/0x2000);
    REQUIRE(std::holds_alternative<Decoded>(res));
    const auto& d = std::get<Decoded>(res);
    // Target = 0x2000 + 2 + (-2) = 0x2000. The loop-back form.
    REQUIRE(d.stmts[0].op == ir::Op{ir::JumpRel{0x2000}});
}

TEST_CASE("decode JMP rel32 consumes 5 bytes and computes absolute target") {
    // E9 00 01 00 00 → jmp +0x100 from end-of-instruction
    // PC = 0x3000, length 5, target = 0x3000 + 5 + 0x100 = 0x3105.
    ir::Ref r = 0;
    const std::vector<std::uint8_t> bytes = {0xE9, 0x00, 0x01, 0x00, 0x00};
    auto res = decoder::decode_one(
        std::span<const std::uint8_t>{bytes}, r, 0x3000);
    REQUIRE(std::holds_alternative<Decoded>(res));
    const auto& d = std::get<Decoded>(res);
    REQUIRE(d.bytes_consumed == 5);
    REQUIRE(d.stmts[0].op == ir::Op{ir::JumpRel{0x3105}});
}

TEST_CASE("decode JE rel8 → CondJumpRel with CondCode::Eq") {
    // 74 0A → je +10
    ir::Ref r = 0;
    const std::vector<std::uint8_t> bytes = {0x74, 0x0A};
    auto res = decoder::decode_one(
        std::span<const std::uint8_t>{bytes}, r, 0x4000);
    REQUIRE(std::holds_alternative<Decoded>(res));
    const auto& d = std::get<Decoded>(res);
    REQUIRE(d.bytes_consumed == 2);
    // target = 0x4000 + 2 + 10 = 0x400C. fallthrough = 0x4000 + 2 = 0x4002.
    REQUIRE(d.stmts[0].op ==
            ir::Op{ir::CondJumpRel{ir::CondCode::Eq, 0x400C, 0x4002}});
}

TEST_CASE("decode JNE rel8 → CondJumpRel with CondCode::Ne") {
    ir::Ref r = 0;
    const std::vector<std::uint8_t> bytes = {0x75, 0x20};  // jne +32
    auto res = decoder::decode_one(
        std::span<const std::uint8_t>{bytes}, r, 0x5000);
    const auto& d = std::get<Decoded>(res);
    REQUIRE(d.stmts[0].op ==
            ir::Op{ir::CondJumpRel{ir::CondCode::Ne, 0x5022, 0x5002}});
}

TEST_CASE("Sequence: MOV rax, 5; MOV rbx, 5; CMP rax, rbx; JE +0; RET") {
    // Absolute PCs, assuming guest_addr = 0x1000:
    //   0x1000..0x1009: mov rax, 5   (10 bytes)
    //   0x100A..0x1013: mov rbx, 5   (10 bytes)
    //   0x1014..0x1016: cmp rax, rbx (3 bytes)
    //   0x1017..0x1018: je +0        (2 bytes, target = 0x1019 = fallthrough)
    //   0x1019..0x1019: ret          (1 byte)
    const std::vector<std::uint8_t> bytes = {
        0x48, 0xB8, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x48, 0xBB, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x48, 0x39, 0xD8,
        0x74, 0x00,
        0xC3,
    };

    ir::Ref next = 0;
    std::size_t cursor = 0;
    std::vector<ir::Stmt> all;
    while (cursor < bytes.size()) {
        const std::uint64_t pc = 0x1000 + cursor;
        auto res = decoder::decode_one(
            std::span<const std::uint8_t>{bytes}.subspan(cursor), next, pc);
        REQUIRE(std::holds_alternative<Decoded>(res));
        auto& d = std::get<Decoded>(res);
        for (auto& s : d.stmts) all.push_back(std::move(s));
        cursor += d.bytes_consumed;
    }

    // Find the CondJumpRel in the stream and verify targets.
    bool saw_cj = false;
    for (const auto& s : all) {
        if (std::holds_alternative<ir::CondJumpRel>(s.op)) {
            const auto& c = std::get<ir::CondJumpRel>(s.op);
            REQUIRE(c.cc == ir::CondCode::Eq);
            REQUIRE(c.fallthrough_guest_pc == 0x1019);
            REQUIRE(c.target_guest_pc == 0x1019);
            saw_cj = true;
        }
    }
    REQUIRE(saw_cj);
}

TEST_CASE("NOP + MOV imm64 + RET sequence decodes cleanly one at a time") {
    // 90                                              ; nop
    // 48 B8 07 00 00 00 00 00 00 00                   ; mov rax, 7
    // C3                                              ; ret
    const std::vector<std::uint8_t> bytes = {
        0x90,
        0x48, 0xB8, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0xC3,
    };
    ir::Ref r = 0;
    std::size_t cursor = 0;
    std::size_t decoded_count = 0;

    while (cursor < bytes.size()) {
        std::span<const std::uint8_t> remaining(
            bytes.data() + cursor, bytes.size() - cursor);
        auto res = decode_one(remaining, r);
        REQUIRE(std::holds_alternative<Decoded>(res));
        auto& d = std::get<Decoded>(res);
        cursor += d.bytes_consumed;
        ++decoded_count;
    }

    REQUIRE(decoded_count == 3);
    REQUIRE(cursor == bytes.size());
    REQUIRE(r == 1);  // only the MOV contributed an SSA ref
}
