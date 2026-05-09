// core/tests/test_decoder.cpp — tests for the MVP x86_64 decoder.
//
// Each test includes the raw bytes of a real x86_64 instruction (verified
// against Intel SDM / objdump) and asserts the IR produced. For compound
// instructions the result order matches the decoder's emission order.

#include <catch2/catch_test_macros.hpp>
#include <array>

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

Decoded decode_ok(std::vector<std::uint8_t> bytes, ir::Ref& ref, std::uint64_t instruction_guest_pc) {
    auto r = decode_one(std::span<const std::uint8_t>{bytes}, ref, instruction_guest_pc);
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

TEST_CASE("decode ADC rax, rbx → same shape as ADD, 3 bytes (carry path placeholder)") {
    // Encoding: 48 11 D8
    //   48 REX.W
    //   11 opcode ADC r/m64, r64
    //   D8 mod=11, reg=011 (rbx, source), rm=000 (rax, destination)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x11, 0xD8}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});
}

TEST_CASE("decode SBB rdx, rcx → same shape as SUB, 3 bytes (borrow path placeholder)") {
    // Encoding: 48 19 CA
    //   48 REX.W
    //   19 opcode SBB r/m64, r64
    //   CA mod=11, reg=001 (rcx, source), rm=010 (rdx, destination)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x19, 0xCA}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rdx, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rcx, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Sub, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rdx, 2u, ir::OpSize::I64}});
}

TEST_CASE("decode TEST rbx, rax → and + CmpFlags zero, 3 bytes (placeholder)") {
    // Encoding: 48 85 C3
    //   48 REX.W
    //   85 opcode TEST r/m64, r64
    //   C3 mod=11, reg=000 (rax, src), rm=011 (rbx, dst)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x85, 0xC3}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 5);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::And, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op == ir::Op{ir::Constant{0u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::CmpFlags{2u, 3u, ir::OpSize::I64}});
}

TEST_CASE("decode INC rax → ADD reg + 1 placeholder, 3 bytes") {
    // Encoding: 48 FF C0
    //   48 REX.W
    //   FF opcode FF /0
    //   C0 mod=11, reg=000 (/0 inc), rm=000 (rax)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0xFF, 0xC0}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});
}

TEST_CASE("decode DEC rax → SUB reg - 1 placeholder, 3 bytes") {
    // Encoding: 48 FF C8
    //   48 REX.W
    //   FF opcode FF /1
    //   C8 mod=11, reg=001 (/1 dec), rm=000 (rax)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0xFF, 0xC8}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Sub, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});
}

TEST_CASE("decode NOT rax → XOR reg with -1 placeholder, 3 bytes") {
    // Encoding: 48 F7 D0
    //   48 REX.W
    //   F7 opcode F7 /2
    //   D0 mod=11, reg=010 (/2 not), rm=000 (rax)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0xF7, 0xD0}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{~0ULL, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Xor, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});
}

TEST_CASE("decode NEG rax → Sub zero - reg placeholder, 3 bytes") {
    // Encoding: 48 F7 D8
    //   48 REX.W
    //   F7 opcode F7 /3
    //   D8 mod=11, reg=011 (/3 neg), rm=000 (rax)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0xF7, 0xD8}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[0].op == ir::Op{ir::Constant{0u, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Sub, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});
}

TEST_CASE("decode MUL rax, rax → RAX = low(RAX*RAX), RDX = umulhi(RAX*RAX), 3 bytes") {
    // Encoding: 48 F7 E0
    //   48 REX.W
    //   F7 opcode F7 /4
    //   E0 mod=11, reg=100 (/4), rm=000 (rax)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0xF7, 0xE0}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 6);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Mul, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::UMulHi, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[5].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rdx, 3u, ir::OpSize::I64}});
}

TEST_CASE("decode IMUL rax, rax → RAX = low(RAX*RAX), RDX = smulhi(RAX*RAX), 3 bytes") {
    // Encoding: 48 F7 E8
    //   48 REX.W
    //   F7 opcode F7 /5
    //   E8 mod=11, reg=101 (/5), rm=000 (rax)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0xF7, 0xE8}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 6);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Mul, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::SMulHi, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[5].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rdx, 3u, ir::OpSize::I64}});
}

TEST_CASE("decode IMUL rax, rbx placeholder → RAX*RBX, 3 bytes") {
    // Encoding: 48 0F AF C3
    //   48 REX.W
    //   0F AF opcode
    //   C3 mod=11, reg=000 (rax, destination), rm=011 (rbx, source)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x0F, 0xAF, 0xC3}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Mul, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});
    REQUIRE(r == 3);
}

TEST_CASE("decode IMUL rax, rbx, -1 placeholder → RBX*-1, sign-extended imm32") {
    // Encoding: 48 69 C3 FF FF FF FF
    //   48 REX.W
    //   69 opcode IMUL r64, r/m64, imm32
    //   C3 mod=11, reg=000 (rax, destination), rm=011 (rbx)
    //   FF FF FF FF = -1 sign-extended to 0xFFFFFFFFFFFFFFFF
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x69, 0xC3, 0xFF, 0xFF, 0xFF, 0xFF}, r);
    REQUIRE(d.bytes_consumed == 7);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op ==
            ir::Op{ir::Constant{0xFFFF'FFFF'FFFF'FFFFULL, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Mul, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});
    REQUIRE(r == 3);
}

TEST_CASE("decode IMUL rax, rbx, 10 placeholder → RBX*10, sign-extended imm8") {
    // Encoding: 48 6B C3 0A
    //   48 REX.W
    //   6B opcode IMUL r64, r/m64, imm8
    //   C3 mod=11, reg=000 (rax, destination), rm=011 (rbx)
    //   0A = +10
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x6B, 0xC3, 0x0A}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{10u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Mul, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});
    REQUIRE(r == 3);
}

TEST_CASE("decode DIV rax, rax → RAX = udiv, RDX = umod, 3 bytes") {
    // Encoding: 48 F7 F0
    //   48 REX.W
    //   F7 opcode F7 /6
    //   F0 mod=11, reg=110 (/6), rm=000 (rax)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0xF7, 0xF0}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 6);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::UDiv, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::UMod, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[5].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rdx, 3u, ir::OpSize::I64}});
    REQUIRE(r == 4);
}

TEST_CASE("decode IDIV rax, rax → RAX = sdiv, RDX = smod, 3 bytes") {
    // Encoding: 48 F7 F8
    //   48 REX.W
    //   F7 opcode F7 /7
    //   F8 mod=11, reg=111 (/7), rm=000 (rax)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0xF7, 0xF8}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 6);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::SDiv, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::SMod, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[5].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rdx, 3u, ir::OpSize::I64}});
    REQUIRE(r == 4);
}

TEST_CASE("decode SHL rax, 3 placeholder via 48 C1 E0 03") {
    // Encoding: 48 C1 E0 03
    //   48 REX.W
    //   C1 opcode SHL r/m64, imm8
    //   E0 mod=11, reg=100 (/4), rm=000 (rax)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0xC1, 0xE0, 0x03}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{3u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Shl, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});
    REQUIRE(r == 3);
}

TEST_CASE("decode ROL rax, 3 placeholder via 48 C1 C0 03") {
    // Encoding: 48 C1 C0 03
    //   48 REX.W
    //   C1 opcode ROL r/m64, imm8
    //   C0 mod=11, reg=000 (/0), rm=000 (rax)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0xC1, 0xC0, 0x03}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{3u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Rol, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});
    REQUIRE(r == 3);
}

TEST_CASE("decode ROR rax, 3 placeholder via 48 C1 C8 03") {
    // Encoding: 48 C1 C8 03
    //   48 REX.W
    //   C1 opcode ROR r/m64, imm8
    //   C8 mod=11, reg=001 (/1), rm=000 (rax)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0xC1, 0xC8, 0x03}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{3u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Ror, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});
    REQUIRE(r == 3);
}

TEST_CASE("decode RCL rax, 3 placeholder via 48 C1 D0 03") {
    // Encoding: 48 C1 D0 03
    //   48 REX.W
    //   C1 opcode RCL r/m64, imm8
    //   D0 mod=11, reg=010 (/2), rm=000 (rax)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0xC1, 0xD0, 0x03}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{3u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Rcl, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});
    REQUIRE(r == 3);
}

TEST_CASE("decode RCR rax, 3 placeholder via 48 C1 D8 03") {
    // Encoding: 48 C1 D8 03
    //   48 REX.W
    //   C1 opcode RCR r/m64, imm8
    //   D8 mod=11, reg=011 (/3), rm=000 (rax)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0xC1, 0xD8, 0x03}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{3u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Rcr, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});
    REQUIRE(r == 3);
}

TEST_CASE("decode BT rax, 3 placeholder via 48 0F BA E0 03") {
    // Encoding: 48 0F BA E0 03
    //   48 REX.W
    //   0F BA opcode BT /4
    //   E0 mod=11, reg=100 (/4), rm=000 (rax)
    //   03 immediate bit index
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x0F, 0xBA, 0xE0, 0x03}, r);
    REQUIRE(d.bytes_consumed == 5);
    REQUIRE(d.stmts.size() == 7);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{3u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::Constant{1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Shl, 3u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op == ir::Op{ir::BinOp{ir::BinOpKind::And, 0u, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[5].op == ir::Op{ir::Constant{0u, ir::OpSize::I64}});
    REQUIRE(d.stmts[6].op == ir::Op{ir::CmpFlags{4u, 5u, ir::OpSize::I64}});
    REQUIRE(r == 6);
}

TEST_CASE("decode BTS rax, 3 placeholder via 48 0F BA E8 03") {
    // Encoding: 48 0F BA E8 03
    //   48 REX.W
    //   0F BA opcode BTS /5
    //   E8 mod=11, reg=101 (/5), rm=000 (rax)
    //   03 immediate bit index
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x0F, 0xBA, 0xE8, 0x03}, r);
    REQUIRE(d.bytes_consumed == 5);
    REQUIRE(d.stmts.size() == 9);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{3u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::Constant{1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Shl, 3u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op == ir::Op{ir::BinOp{ir::BinOpKind::And, 0u, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[5].op == ir::Op{ir::BinOp{ir::BinOpKind::Or, 0u, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[6].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 6u, ir::OpSize::I64}});
    REQUIRE(d.stmts[7].op == ir::Op{ir::Constant{0u, ir::OpSize::I64}});
    REQUIRE(d.stmts[8].op == ir::Op{ir::CmpFlags{4u, 5u, ir::OpSize::I64}});
    REQUIRE(r == 7);
}

TEST_CASE("decode BTR rax, 3 placeholder via 48 0F BA F0 03") {
    // Encoding: 48 0F BA F0 03
    //   48 REX.W
    //   0F BA opcode BTR /6
    //   F0 mod=11, reg=110 (/6), rm=000 (rax)
    //   03 immediate bit index
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x0F, 0xBA, 0xF0, 0x03}, r);
    REQUIRE(d.bytes_consumed == 5);
    REQUIRE(d.stmts.size() == 11);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{3u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::Constant{1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Shl, 3u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op == ir::Op{ir::BinOp{ir::BinOpKind::And, 0u, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[5].op == ir::Op{ir::Constant{~0ULL, ir::OpSize::I64}});
    REQUIRE(d.stmts[6].op == ir::Op{ir::BinOp{ir::BinOpKind::Xor, 2u, 6u, ir::OpSize::I64}});
    REQUIRE(d.stmts[7].op == ir::Op{ir::BinOp{ir::BinOpKind::And, 0u, 6u, ir::OpSize::I64}});
    REQUIRE(d.stmts[8].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 7u, ir::OpSize::I64}});
    REQUIRE(d.stmts[9].op == ir::Op{ir::Constant{0u, ir::OpSize::I64}});
    REQUIRE(d.stmts[10].op == ir::Op{ir::CmpFlags{4u, 5u, ir::OpSize::I64}});
    REQUIRE(r == 8);
}

TEST_CASE("decode BTC rax, 3 placeholder via 48 0F BA F8 03") {
    // Encoding: 48 0F BA F8 03
    //   48 REX.W
    //   0F BA opcode BTC /7
    //   F8 mod=11, reg=111 (/7), rm=000 (rax)
    //   03 immediate bit index
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x0F, 0xBA, 0xF8, 0x03}, r);
    REQUIRE(d.bytes_consumed == 5);
    REQUIRE(d.stmts.size() == 9);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{3u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::Constant{1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Shl, 3u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op == ir::Op{ir::BinOp{ir::BinOpKind::And, 0u, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[5].op == ir::Op{ir::BinOp{ir::BinOpKind::Xor, 0u, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[6].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 6u, ir::OpSize::I64}});
    REQUIRE(d.stmts[7].op == ir::Op{ir::Constant{0u, ir::OpSize::I64}});
    REQUIRE(d.stmts[8].op == ir::Op{ir::CmpFlags{4u, 5u, ir::OpSize::I64}});
    REQUIRE(r == 7);
}

TEST_CASE("decode BSF rax, rax placeholder via 48 0F BC C0") {
    // Encoding: 48 0F BC C0
    //   48 REX.W
    //   0F BC opcode BSF /r
    //   C0 mod=11, reg=000 (rax destination), rm=000 (rax source)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x0F, 0xBC, 0xC0}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{0u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op == ir::Op{ir::CmpFlags{0u, 1u, ir::OpSize::I64}});
    REQUIRE(r == 2);
}

TEST_CASE("decode BSR rax, rax placeholder via 48 0F BD C0") {
    // Encoding: 48 0F BD C0
    //   48 REX.W
    //   0F BD opcode BSR /r
    //   C0 mod=11, reg=000 (rax destination), rm=000 (rax source)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x0F, 0xBD, 0xC0}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{0u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op == ir::Op{ir::CmpFlags{0u, 1u, ir::OpSize::I64}});
    REQUIRE(r == 2);
}

TEST_CASE("decode LZCNT rax, rax placeholder via F3 48 0F BD C0") {
    // Encoding: F3 48 0F BD C0
    //   F3 prefix for LZCNT
    //   48 REX.W
    //   0F BD opcode BD /r (normally BSR, with F3 => LZCNT)
    //   C0 mod=11, reg=000 (rax destination), rm=000 (rax source)
    ir::Ref r = 0;
    auto d = decode_ok({0xF3, 0x48, 0x0F, 0xBD, 0xC0}, r);
    REQUIRE(d.bytes_consumed == 5);
    REQUIRE(d.stmts.size() == 3);
    REQUIRE(std::holds_alternative<ir::Lzcnt>(d.stmts[1].op));
}

TEST_CASE("decode TZCNT rax, rax via F3 48 0F BC C0 — real TZCNT (F2-IR-045)") {
    ir::Ref r = 0;
    auto d = decode_ok({0xF3, 0x48, 0x0F, 0xBC, 0xC0}, r);
    REQUIRE(d.bytes_consumed == 5);
    REQUIRE(d.stmts.size() == 3);
    REQUIRE(std::holds_alternative<ir::Tzcnt>(d.stmts[1].op));
}

TEST_CASE("decode POPCNT rax, rax via F3 48 0F B8 C0 — real POPCNT (F2-IR-044)") {
    // Encoding: F3 48 0F B8 C0
    //   F3 prefix for POPCNT
    //   48 REX.W
    //   0F B8 opcode B8 /r (with F3 prefix → POPCNT)
    //   C0 mod=11, reg=000 (rax destination), rm=000 (rax source)
    // Now produces 3-stmt sequence: LoadReg → Popcnt → StoreReg.
    ir::Ref r = 0;
    auto d = decode_ok({0xF3, 0x48, 0x0F, 0xB8, 0xC0}, r);
    REQUIRE(d.bytes_consumed == 5);
    REQUIRE(d.stmts.size() == 3);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(std::holds_alternative<ir::Popcnt>(d.stmts[1].op));
    REQUIRE(std::holds_alternative<ir::StoreReg>(d.stmts[2].op));
}

TEST_CASE("decode PUSH rax placeholder via 50") {
    // Encoding: 50
    //   50 opcode PUSH r64 (rd = 0 => rax)
    ir::Ref r = 0;
    auto d = decode_ok({0x50}, r);
    REQUIRE(d.bytes_consumed == 1);
    REQUIRE(d.stmts.size() == 6);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rsp, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{8u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Sub, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::StoreMemTSO{2u, 3u, ir::OpSize::I64}});
    REQUIRE(d.stmts[5].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rsp, 2u, ir::OpSize::I64}});
    REQUIRE(r == 4);
}

TEST_CASE("decode POP rax placeholder via 58") {
    // Encoding: 58
    //   58 opcode POP r64 (rd = 0 => rax)
    ir::Ref r = 0;
    auto d = decode_ok({0x58}, r);
    REQUIRE(d.bytes_consumed == 1);
    REQUIRE(d.stmts.size() == 6);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rsp, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadMemTSO{0u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::Constant{8u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Add, 0u, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[5].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rsp, 3u, ir::OpSize::I64}});
    REQUIRE(r == 4);
}

TEST_CASE("decode PUSH imm8 -1 placeholder via 6A FF") {
    // Encoding: 6A FF
    //   6A opcode PUSH imm8 with immediate -1 (sign-extended to 64 bits)
    ir::Ref r = 0;
    auto d = decode_ok({0x6A, 0xFF}, r);
    REQUIRE(d.bytes_consumed == 2);
    REQUIRE(d.stmts.size() == 6);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rsp, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{8u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Sub, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op == ir::Op{ir::Constant{0xFFFF'FFFF'FFFF'FFFFULL, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::StoreMemTSO{2u, 3u, ir::OpSize::I64}});
    REQUIRE(d.stmts[5].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rsp, 2u, ir::OpSize::I64}});
    REQUIRE(r == 4);
}

TEST_CASE("decode PUSH imm32 0x12345678 placeholder via 68 78563412") {
    // Encoding: 68 78 56 34 12
    //   68 opcode PUSH imm32 with sign-extension path
    ir::Ref r = 0;
    auto d = decode_ok({0x68, 0x78, 0x56, 0x34, 0x12}, r);
    REQUIRE(d.bytes_consumed == 5);
    REQUIRE(d.stmts.size() == 6);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rsp, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{8u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Sub, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op == ir::Op{ir::Constant{0x1234'5678ULL, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::StoreMemTSO{2u, 3u, ir::OpSize::I64}});
    REQUIRE(d.stmts[5].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rsp, 2u, ir::OpSize::I64}});
    REQUIRE(r == 4);
}

TEST_CASE("decode PUSHFQ placeholder via 9C") {
    // Encoding: 9C
    //   9C opcode PUSHFQ placeholder (push zero as flags placeholder)
    ir::Ref r = 0;
    auto d = decode_ok({0x9C}, r);
    REQUIRE(d.bytes_consumed == 1);
    REQUIRE(d.stmts.size() == 6);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rsp, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{8u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Sub, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op == ir::Op{ir::Constant{0u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::StoreMemTSO{2u, 3u, ir::OpSize::I64}});
    REQUIRE(d.stmts[5].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rsp, 2u, ir::OpSize::I64}});
    REQUIRE(r == 4);
}

TEST_CASE("decode POPFQ placeholder via 9D") {
    // Encoding: 9D
    //   9D opcode POPFQ placeholder (pop and discard flags placeholder)
    ir::Ref r = 0;
    auto d = decode_ok({0x9D}, r);
    REQUIRE(d.bytes_consumed == 1);
    REQUIRE(d.stmts.size() == 5);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rsp, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadMemTSO{0u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::Constant{8u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Add, 0u, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rsp, 3u, ir::OpSize::I64}});
    REQUIRE(r == 4);
}

TEST_CASE("decode LEA rax, [rax + 8] placeholder via 48 8D 40 08") {
    // Encoding: 48 8D 40 08
    //   48 REX.W
    //   8D opcode LEA r64, [m]
    //   40 mod=01, reg=000 (rax destination), rm=000 (rax source), disp8=8
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x8D, 0x40, 0x08}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{8u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});
    REQUIRE(r == 3);
}

TEST_CASE("decode MOVSX rbx, al → sign-extend I8 via 48 0F BE C8") {
    // Encoding: 48 0F BE C8
    //   48 REX.W
    //   0F BE /r is MOVSX r64, r/m8
    //   C8 mod=11, reg=001 (rcx destination), rm=000 (rax/al source)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x0F, 0xBE, 0xC8}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 5);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I8}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{56u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Shl, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Sar, 2u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rcx, 3u, ir::OpSize::I64}});
    REQUIRE(r == 4);
}

TEST_CASE("decode MOVSX rcx, ax → sign-extend I16 via 48 0F BF C8") {
    // Encoding: 48 0F BF C8
    //   48 REX.W
    //   0F BF /r is MOVSX r64, r/m16
    //   C8 mod=11, reg=001 (rcx destination), rm=000 (rax/ax source)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x0F, 0xBF, 0xC8}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 5);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I16}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{48u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Shl, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Sar, 2u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rcx, 3u, ir::OpSize::I64}});
    REQUIRE(r == 4);
}

TEST_CASE("decode MOVSXD rcx, eax → sign-extend I32 via 48 63 C8") {
    // Encoding: 48 63 C8
    //   48 REX.W
    //   63 /r is MOVSXD r64, r/m32
    //   C8 mod=11, reg=001 (rcx destination), rm=000 (rax/eax source)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x63, 0xC8}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 5);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I32}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{32u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Shl, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Sar, 2u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rcx, 3u, ir::OpSize::I64}});
    REQUIRE(r == 4);
}

TEST_CASE("decode MOVZX rbx, al → zero-extend I8 via 48 0F B6 C8") {
    // Encoding: 48 0F B6 C8
    //   48 REX.W
    //   0F B6 /r is MOVZX r64, r/m8
    //   C8 mod=11, reg=001 (rcx destination), rm=000 (rax/al source)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x0F, 0xB6, 0xC8}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 5);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I8}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{56u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Shl, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Shr, 2u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rcx, 3u, ir::OpSize::I64}});
    REQUIRE(r == 4);
}

TEST_CASE("decode MOVZX rcx, ax → zero-extend I16 via 48 0F B7 C8") {
    // Encoding: 48 0F B7 C8
    //   48 REX.W
    //   0F B7 /r is MOVZX r64, r/m16
    //   C8 mod=11, reg=001 (rcx destination), rm=000 (rax/ax source)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x0F, 0xB7, 0xC8}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 5);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I16}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{48u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Shl, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Shr, 2u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rcx, 3u, ir::OpSize::I64}});
    REQUIRE(r == 4);
}

TEST_CASE("decode CBW via 66 98 → sign-extend I8 to I16 in AL/AX") {
    // Encoding: 66 98
    //   66 operand-size override
    //   98 opcode CBW when operating width is 16-bit (AL → AX)
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x98}, r);
    REQUIRE(d.bytes_consumed == 2);
    REQUIRE(d.stmts.size() == 5);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I8}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{56u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Shl, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Sar, 2u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 3u, ir::OpSize::I16}});
    REQUIRE(r == 4);
}

TEST_CASE("decode CWDE via 98 → sign-extend I16 to I32 in RAX") {
    // Encoding: 98
    //   98 opcode is CWDE when operand size is 32-bit
    ir::Ref r = 0;
    auto d = decode_ok({0x98}, r);
    REQUIRE(d.bytes_consumed == 1);
    REQUIRE(d.stmts.size() == 5);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I16}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{48u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Shl, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Sar, 2u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 3u, ir::OpSize::I32}});
    REQUIRE(r == 4);
}

TEST_CASE("decode CDQE via 48 98 → sign-extend I32 to I64 in RAX") {
    // Encoding: 48 98
    //   48 REX.W
    //   98 opcode is CDQE when REX.W is set (EAX → RAX)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x98}, r);
    REQUIRE(d.bytes_consumed == 2);
    REQUIRE(d.stmts.size() == 5);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I32}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{32u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Shl, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Sar, 2u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 3u, ir::OpSize::I64}});
    REQUIRE(r == 4);
}

TEST_CASE("decode CWD via 66 99 → AX sign-bit replicated into DX") {
    // Encoding: 66 99
    //   66 operand-size override
    //   99 opcode is CWD when operand size is 16-bit
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x99}, r);
    REQUIRE(d.bytes_consumed == 2);
    REQUIRE(d.stmts.size() == 5);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I16}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{15u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Sar, 0u, 1u, ir::OpSize::I16}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 0u, ir::OpSize::I16}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rdx, 2u, ir::OpSize::I16}});
    REQUIRE(r == 3);
}

TEST_CASE("decode CDQ via 99 → EAX sign-bit replicated into EDX") {
    // Encoding: 99
    //   99 opcode is CDQ when operand size is 32-bit
    ir::Ref r = 0;
    auto d = decode_ok({0x99}, r);
    REQUIRE(d.bytes_consumed == 1);
    REQUIRE(d.stmts.size() == 5);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I32}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{31u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Sar, 0u, 1u, ir::OpSize::I32}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 0u, ir::OpSize::I32}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rdx, 2u, ir::OpSize::I32}});
    REQUIRE(r == 3);
}

TEST_CASE("decode CQO via 48 99 → RAX sign-bit replicated into RDX") {
    // Encoding: 48 99
    //   48 REX.W
    //   99 opcode is CQO when REX.W is set (RAX → RDX:RAX)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x99}, r);
    REQUIRE(d.bytes_consumed == 2);
    REQUIRE(d.stmts.size() == 5);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{63u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Sar, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 0u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rdx, 2u, ir::OpSize::I64}});
    REQUIRE(r == 3);
}

TEST_CASE("decode MOVSX rax, byte ptr [rbx] → sign-extend I8 via 48 0F BE 03") {
    // Encoding: 48 0F BE 03
    //   0F BE /r is MOVSX r64, r/m8
    //   03 mod=00, reg=000 (rax destination), rm=011 (rbx source)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x0F, 0xBE, 0x03}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 6);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadMemTSO{0u, ir::OpSize::I8}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::Constant{56u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Shl, 1u, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Sar, 3u, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[5].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 4u, ir::OpSize::I64}});
    REQUIRE(r == 5);
}

TEST_CASE("decode MOVSX rax, word ptr [rbx] → sign-extend I16 via 48 0F BF 03") {
    // Encoding: 48 0F BF 03
    //   0F BF /r is MOVSX r64, r/m16
    //   03 mod=00, reg=000 (rax destination), rm=011 (rbx source)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x0F, 0xBF, 0x03}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 6);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadMemTSO{0u, ir::OpSize::I16}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::Constant{48u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Shl, 1u, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].result == std::optional<ir::Ref>(3u));
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Sar, 3u, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[5].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 4u, ir::OpSize::I64}});
    REQUIRE(r == 5);
}

TEST_CASE("decode MOVSXD rax, dword ptr [rbx] → sign-extend I32 via 48 63 03") {
    // Encoding: 48 63 03
    //   63 /r is MOVSXD r64, r/m32
    //   03 mod=00, reg=000 (rax destination), rm=011 (rbx source)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x63, 0x03}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 6);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadMemTSO{0u, ir::OpSize::I32}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::Constant{32u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Shl, 1u, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Sar, 3u, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[5].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 4u, ir::OpSize::I64}});
    REQUIRE(r == 5);
}

TEST_CASE("decode MOVZX rax, byte ptr [rbx] → zero-extend I8 via 48 0F B6 03") {
    // Encoding: 48 0F B6 03
    //   0F B6 /r is MOVZX r64, r/m8
    //   03 mod=00, reg=000 (rax destination), rm=011 (rbx source)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x0F, 0xB6, 0x03}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 6);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadMemTSO{0u, ir::OpSize::I8}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::Constant{56u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Shl, 1u, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Shr, 3u, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[5].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 4u, ir::OpSize::I64}});
    REQUIRE(r == 5);
}

TEST_CASE("decode MOVZX rax, word ptr [rbx] → zero-extend I16 via 48 0F B7 03") {
    // Encoding: 48 0F B7 03
    //   0F B7 /r is MOVZX r64, r/m16
    //   03 mod=00, reg=000 (rax destination), rm=011 (rbx source)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x0F, 0xB7, 0x03}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 6);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadMemTSO{0u, ir::OpSize::I16}});
    REQUIRE(d.stmts[1].result == std::optional<ir::Ref>(1u));
    REQUIRE(d.stmts[2].op == ir::Op{ir::Constant{48u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Shl, 1u, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Shr, 3u, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[5].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 4u, ir::OpSize::I64}});
    REQUIRE(r == 5);
}

TEST_CASE("decode SHR rax, 3 placeholder via 48 C1 E8 03") {
    // Encoding: 48 C1 E8 03
    //   48 REX.W
    //   C1 opcode SHR r/m64, imm8
    //   E8 mod=11, reg=101 (/5), rm=000 (rax)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0xC1, 0xE8, 0x03}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{3u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Shr, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});
    REQUIRE(r == 3);
}

TEST_CASE("decode SAR rax, 3 placeholder via 48 C1 F8 03") {
    // Encoding: 48 C1 F8 03
    //   48 REX.W
    //   C1 opcode SAR r/m64, imm8
    //   F8 mod=11, reg=111 (/7), rm=000 (rax)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0xC1, 0xF8, 0x03}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{3u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Sar, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});
    REQUIRE(r == 3);
}

TEST_CASE("decode SHL rax, cl placeholder via 48 D3 E0") {
    // Encoding: 48 D3 E0
    //   48 REX.W
    //   D3 opcode SHL r/m64, CL
    //   E0 mod=11, reg=100 (/4), rm=000 (rax)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0xD3, 0xE0}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rcx, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Shl, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});
    REQUIRE(r == 3);
}

TEST_CASE("decode ROL rax, cl placeholder via 48 D3 C0") {
    // Encoding: 48 D3 C0
    //   48 REX.W
    //   D3 opcode ROL r/m64, CL
    //   C0 mod=11, reg=000 (/0), rm=000 (rax)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0xD3, 0xC0}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rcx, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Rol, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});
    REQUIRE(r == 3);
}

TEST_CASE("decode ROR rax, cl placeholder via 48 D3 C8") {
    // Encoding: 48 D3 C8
    //   48 REX.W
    //   D3 opcode ROR r/m64, CL
    //   C8 mod=11, reg=001 (/1), rm=000 (rax)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0xD3, 0xC8}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rcx, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Ror, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});
    REQUIRE(r == 3);
}

TEST_CASE("decode RCL rax, cl placeholder vía 48 D3 D0") {
    // Encoding: 48 D3 D0
    //   48 REX.W
    //   D3 opcode RCL r/m64, CL
    //   D0 mod=11, reg=010 (/2), rm=000 (rax)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0xD3, 0xD0}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rcx, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Rcl, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});
    REQUIRE(r == 3);
}

TEST_CASE("decode RCR rax, cl placeholder via 48 D3 D8") {
    // Encoding: 48 D3 D8
    //   48 REX.W
    //   D3 opcode RCR r/m64, CL
    //   D8 mod=11, reg=011 (/3), rm=000 (rax)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0xD3, 0xD8}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rcx, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Rcr, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});
    REQUIRE(r == 3);
}

TEST_CASE("decode SHR rax, cl placeholder via 48 D3 E8") {
    // Encoding: 48 D3 E8
    //   48 REX.W
    //   D3 opcode SHR r/m64, CL
    //   E8 mod=11, reg=101 (/5), rm=000 (rax)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0xD3, 0xE8}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rcx, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Shr, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});
    REQUIRE(r == 3);
}

TEST_CASE("decode SAR rax, cl placeholder via 48 D3 F8") {
    // Encoding: 48 D3 F8
    //   48 REX.W
    //   D3 opcode SAR r/m64, CL
    //   F8 mod=11, reg=111 (/7), rm=000 (rax)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0xD3, 0xF8}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rcx, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Sar, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});
    REQUIRE(r == 3);
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
    auto res = decode_any({0x62}, r);
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

TEST_CASE("decode ADD rax, r11 via REX.R extension (4C 01 D8)") {
    // 4C 01 D8  →  ADD rax, r11  (REX.R=1 extends reg field to r11).
    // Codex's SIB/REX refactor now supports extended registers.
    ir::Ref r = 0;
    auto d = decode_ok({0x4C, 0x01, 0xD8}, r);
    REQUIRE(d.bytes_consumed == 3);
    // Should produce LoadReg + LoadReg + BinOp + StoreReg (the ALU shape).
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(std::holds_alternative<ir::LoadReg>(d.stmts[0].op));
    REQUIRE(std::get<ir::LoadReg>(d.stmts[0].op).reg == ir::Gpr::Rax);
    REQUIRE(std::holds_alternative<ir::LoadReg>(d.stmts[1].op));
    REQUIRE(std::get<ir::LoadReg>(d.stmts[1].op).reg == ir::Gpr::R11);
}

TEST_CASE("decode MOV r8, rbx via REX.B extension (49 89 D8)") {
    ir::Ref r = 0;
    auto d = decode_ok({0x49, 0x89, 0xD8}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 2);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::StoreReg{ir::Gpr::R8, 0u, ir::OpSize::I64}});
}

TEST_CASE("decode MOV r8, imm64 via REX.B opcode-register extension") {
    ir::Ref r = 0;
    auto d = decode_ok(
        {0x49, 0xB8, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11}, r);
    REQUIRE(d.bytes_consumed == 10);
    REQUIRE(d.stmts.size() == 2);
    REQUIRE(d.stmts[0].op ==
            ir::Op{ir::Constant{0x1122'3344'5566'7788ULL, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::StoreReg{ir::Gpr::R8, 0u, ir::OpSize::I64}});
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

TEST_CASE("decode XCHG rax, rbx via 48 87 C3 as swap sequence") {
    // 48 87 C3
    //   87 opcode XCHG r/m64, r64
    //   C3 mod=11, reg=000 (rax), rm=011 (rbx)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x87, 0xC3}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::StoreReg{ir::Gpr::Rbx, 0u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op == ir::Op{ir::StoreReg{ir::Gpr::Rax, 1u, ir::OpSize::I64}});
}

TEST_CASE("decode XCHG rax, qword ptr [rbx + 0x10] via 48 87 43 10") {
    // 48 87 43 10
    //   87 opcode XCHG r/m64, r64
    //   43 mod=01, reg=000 (rax), rm=011 (rbx), disp8=0x10
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x87, 0x43, 0x10}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 7);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::Constant{0x10u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op == ir::Op{ir::BinOp{ir::BinOpKind::Add, 1u, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op == ir::Op{ir::LoadMemTSO{3u, ir::OpSize::I64}});
    REQUIRE(d.stmts[5].op == ir::Op{ir::StoreReg{ir::Gpr::Rax, 4u, ir::OpSize::I64}});
    REQUIRE(d.stmts[6].op == ir::Op{ir::StoreMemTSO{3u, 0u, ir::OpSize::I64}});
}

TEST_CASE("decode CMPXCHG rbx, rcx via 48 0F B1 CB as compare-exchange sequence") {
    // 48 0F B1 CB
    //   B1 opcode CMPXCHG r/m64, r64
    //   CB mod=11, reg=001 (rcx), rm=011 (rbx)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x0F, 0xB1, 0xCB}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 8);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rcx, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op == ir::Op{ir::CmpFlags{0u, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::Select{ir::CondCode::Eq, 1u, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[5].op ==
            ir::Op{ir::Select{ir::CondCode::Eq, 0u, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[6].op == ir::Op{ir::StoreReg{ir::Gpr::Rbx, 3u, ir::OpSize::I64}});
    REQUIRE(d.stmts[7].op == ir::Op{ir::StoreReg{ir::Gpr::Rax, 4u, ir::OpSize::I64}});
}

TEST_CASE("decode CMPXCHG [rbx + 0x10], rcx via 48 0F B1 4B 10") {
    // 48 0F B1 4B 10
    //   B1 opcode CMPXCHG r/m64, r64
    //   4B mod=01, reg=001 (rcx), rm=011 (rbx), disp8=0x10
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x0F, 0xB1, 0x4B, 0x10}, r);
    REQUIRE(d.bytes_consumed == 5);
    REQUIRE(d.stmts.size() == 11);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rcx, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op == ir::Op{ir::Constant{0x10u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Add, 2u, 3u, ir::OpSize::I64}});
    REQUIRE(d.stmts[5].op == ir::Op{ir::LoadMemTSO{4u, ir::OpSize::I64}});
    REQUIRE(d.stmts[6].op == ir::Op{ir::CmpFlags{0u, 5u, ir::OpSize::I64}});
    REQUIRE(d.stmts[7].op ==
            ir::Op{ir::Select{ir::CondCode::Eq, 1u, 5u, ir::OpSize::I64}});
    REQUIRE(d.stmts[8].op ==
            ir::Op{ir::Select{ir::CondCode::Eq, 0u, 5u, ir::OpSize::I64}});
    REQUIRE(d.stmts[9].op == ir::Op{ir::StoreMemTSO{4u, 6u, ir::OpSize::I64}});
    REQUIRE(d.stmts[10].op == ir::Op{ir::StoreReg{ir::Gpr::Rax, 7u, ir::OpSize::I64}});
}

TEST_CASE("decode CMPXCHG16B [rsi] via 48 0F C7 0E as 128-bit compare-exchange placeholder") {
    // 48 0F C7 0E
    //   C7 /1 opcode CMPXCHG16B m128
    //   0E mod=00, reg=001 (/1), rm=110 ([rsi])
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x0F, 0xC7, 0x0E}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 22);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rsi, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{0u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op == ir::Op{ir::LoadReg{ir::Gpr::Rdx, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[5].op == ir::Op{ir::LoadReg{ir::Gpr::Rcx, ir::OpSize::I64}});
    REQUIRE(d.stmts[6].op == ir::Op{ir::LoadMemTSO{0u, ir::OpSize::I64}});
    REQUIRE(d.stmts[7].op == ir::Op{ir::Constant{8u, ir::OpSize::I64}});
    REQUIRE(d.stmts[8].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Add, 0u, 7u, ir::OpSize::I64}});
    REQUIRE(d.stmts[9].op == ir::Op{ir::LoadMemTSO{8u, ir::OpSize::I64}});
    REQUIRE(d.stmts[10].op ==
            ir::Op{ir::Compare{ir::CondCode::Eq, 6u, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[11].op ==
            ir::Op{ir::Compare{ir::CondCode::Eq, 9u, 3u, ir::OpSize::I64}});
    REQUIRE(d.stmts[12].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::And, 10u, 11u, ir::OpSize::I64}});
    REQUIRE(d.stmts[13].op == ir::Op{ir::CmpFlags{12u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[14].op ==
            ir::Op{ir::Select{ir::CondCode::Ne, 4u, 6u, ir::OpSize::I64}});
    REQUIRE(d.stmts[15].op ==
            ir::Op{ir::Select{ir::CondCode::Ne, 5u, 9u, ir::OpSize::I64}});
    REQUIRE(d.stmts[16].op == ir::Op{ir::StoreMemTSO{0u, 13u, ir::OpSize::I64}});
    REQUIRE(d.stmts[17].op == ir::Op{ir::StoreMemTSO{8u, 14u, ir::OpSize::I64}});
    REQUIRE(d.stmts[18].op ==
            ir::Op{ir::Select{ir::CondCode::Ne, 2u, 6u, ir::OpSize::I64}});
    REQUIRE(d.stmts[19].op ==
            ir::Op{ir::Select{ir::CondCode::Ne, 3u, 9u, ir::OpSize::I64}});
    REQUIRE(d.stmts[20].op == ir::Op{ir::StoreReg{ir::Gpr::Rax, 15u, ir::OpSize::I64}});
    REQUIRE(d.stmts[21].op == ir::Op{ir::StoreReg{ir::Gpr::Rdx, 16u, ir::OpSize::I64}});
}

TEST_CASE("decode XADD rbx, rcx via 48 0F C1 CB as exchange-add sequence") {
    // 48 0F C1 CB
    //   C1 opcode XADD r/m64, r64
    //   CB mod=11, reg=001 (rcx), rm=011 (rbx)
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x0F, 0xC1, 0xCB}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 5);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rcx, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Add, 1u, 0u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op == ir::Op{ir::StoreReg{ir::Gpr::Rbx, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op == ir::Op{ir::StoreReg{ir::Gpr::Rcx, 1u, ir::OpSize::I64}});
}

TEST_CASE("decode XADD [rbx + 0x10], rcx via 48 0F C1 4B 10") {
    // 48 0F C1 4B 10
    //   C1 opcode XADD r/m64, r64
    //   4B mod=01, reg=001 (rcx), rm=011 (rbx), disp8=0x10
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x0F, 0xC1, 0x4B, 0x10}, r);
    REQUIRE(d.bytes_consumed == 5);
    REQUIRE(d.stmts.size() == 8);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rcx, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::Constant{0x10u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Add, 2u, 3u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op == ir::Op{ir::LoadMemTSO{4u, ir::OpSize::I64}});
    REQUIRE(d.stmts[5].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Add, 1u, 0u, ir::OpSize::I64}});
    REQUIRE(d.stmts[6].op == ir::Op{ir::StoreMemTSO{4u, 5u, ir::OpSize::I64}});
    REQUIRE(d.stmts[7].op == ir::Op{ir::StoreReg{ir::Gpr::Rcx, 1u, ir::OpSize::I64}});
}

TEST_CASE("decode LOCK CMPXCHG [rbx + 0x10], rcx reuses the same IR") {
    ir::Ref plain_ref = 0;
    ir::Ref lock_ref = 0;
    auto plain = decode_ok({0x48, 0x0F, 0xB1, 0x4B, 0x10}, plain_ref);
    auto locked = decode_ok({0xF0, 0x48, 0x0F, 0xB1, 0x4B, 0x10}, lock_ref);
    REQUIRE(locked.bytes_consumed == 6);
    REQUIRE(locked.stmts == plain.stmts);
    REQUIRE(lock_ref == plain_ref);
}

TEST_CASE("decode XACQUIRE LOCK CMPXCHG [rbx + 0x10], rcx ignores the HLE hint") {
    ir::Ref plain_ref = 0;
    ir::Ref hinted_ref = 0;
    auto plain = decode_ok({0xF0, 0x48, 0x0F, 0xB1, 0x4B, 0x10}, plain_ref);
    auto hinted = decode_ok({0xF2, 0xF0, 0x48, 0x0F, 0xB1, 0x4B, 0x10}, hinted_ref);
    REQUIRE(hinted.bytes_consumed == 7);
    REQUIRE(hinted.stmts == plain.stmts);
    REQUIRE(hinted_ref == plain_ref);
}

TEST_CASE("decode XRELEASE LOCK XADD [rbx + 0x10], rcx ignores the HLE hint") {
    ir::Ref plain_ref = 0;
    ir::Ref hinted_ref = 0;
    auto plain = decode_ok({0xF0, 0x48, 0x0F, 0xC1, 0x4B, 0x10}, plain_ref);
    auto hinted = decode_ok({0xF3, 0xF0, 0x48, 0x0F, 0xC1, 0x4B, 0x10}, hinted_ref);
    REQUIRE(hinted.bytes_consumed == 7);
    REQUIRE(hinted.stmts == plain.stmts);
    REQUIRE(hinted_ref == plain_ref);
}

TEST_CASE("Error: XACQUIRE without LOCK on XADD is rejected") {
    ir::Ref r = 0;
    auto res = decode_any({0xF2, 0x48, 0x0F, 0xC1, 0x4B, 0x10}, r);
    REQUIRE(std::holds_alternative<DecodeError>(res));
    REQUIRE(std::get<DecodeError>(res) == DecodeError::UnsupportedEncoding);
}

TEST_CASE("decode STOSB via AA stores AL and advances RDI by 1") {
    ir::Ref r = 0;
    auto d = decode_ok({0xAA}, r);
    REQUIRE(d.bytes_consumed == 1);
    REQUIRE(d.stmts.size() == 6);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I8}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rdi, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::StoreMemTSO{1u, 0u, ir::OpSize::I8}});
    REQUIRE(d.stmts[3].op == ir::Op{ir::Constant{1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Add, 1u, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[5].op == ir::Op{ir::StoreReg{ir::Gpr::Rdi, 3u, ir::OpSize::I64}});
}

TEST_CASE("decode STOSW via 66 AB stores AX and advances RDI by 2") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0xAB}, r);
    REQUIRE(d.bytes_consumed == 2);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I16}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::StoreMemTSO{1u, 0u, ir::OpSize::I16}});
    REQUIRE(d.stmts[3].op == ir::Op{ir::Constant{2u, ir::OpSize::I64}});
}

TEST_CASE("decode STOSD via AB stores EAX and advances RDI by 4") {
    ir::Ref r = 0;
    auto d = decode_ok({0xAB}, r);
    REQUIRE(d.bytes_consumed == 1);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I32}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::StoreMemTSO{1u, 0u, ir::OpSize::I32}});
    REQUIRE(d.stmts[3].op == ir::Op{ir::Constant{4u, ir::OpSize::I64}});
}

TEST_CASE("decode STOSQ via 48 AB stores RAX and advances RDI by 8") {
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0xAB}, r);
    REQUIRE(d.bytes_consumed == 2);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::StoreMemTSO{1u, 0u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op == ir::Op{ir::Constant{8u, ir::OpSize::I64}});
}

TEST_CASE("decode MOVSB via A4 copies byte [RSI] to [RDI] and advances both pointers") {
    ir::Ref r = 0;
    auto d = decode_ok({0xA4}, r);
    REQUIRE(d.bytes_consumed == 1);
    REQUIRE(d.stmts.size() == 9);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rsi, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadMemTSO{0u, ir::OpSize::I8}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::LoadReg{ir::Gpr::Rdi, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op == ir::Op{ir::StoreMemTSO{2u, 1u, ir::OpSize::I8}});
    REQUIRE(d.stmts[4].op == ir::Op{ir::Constant{1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[5].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Add, 0u, 3u, ir::OpSize::I64}});
    REQUIRE(d.stmts[6].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Add, 2u, 3u, ir::OpSize::I64}});
    REQUIRE(d.stmts[7].op == ir::Op{ir::StoreReg{ir::Gpr::Rsi, 4u, ir::OpSize::I64}});
    REQUIRE(d.stmts[8].op == ir::Op{ir::StoreReg{ir::Gpr::Rdi, 5u, ir::OpSize::I64}});
}

TEST_CASE("decode MOVSQ via 48 A5 copies qword [RSI] to [RDI] and advances by 8") {
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0xA5}, r);
    REQUIRE(d.bytes_consumed == 2);
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadMemTSO{0u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op == ir::Op{ir::StoreMemTSO{2u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op == ir::Op{ir::Constant{8u, ir::OpSize::I64}});
}

TEST_CASE("decode CMPSB via A6 compares [RSI] and [RDI] then advances both pointers") {
    ir::Ref r = 0;
    auto d = decode_ok({0xA6}, r);
    REQUIRE(d.bytes_consumed == 1);
    REQUIRE(d.stmts.size() == 10);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rsi, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadMemTSO{0u, ir::OpSize::I8}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::LoadReg{ir::Gpr::Rdi, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op == ir::Op{ir::LoadMemTSO{2u, ir::OpSize::I8}});
    REQUIRE(d.stmts[4].op == ir::Op{ir::CmpFlags{1u, 3u, ir::OpSize::I8}});
    REQUIRE(d.stmts[5].op == ir::Op{ir::Constant{1u, ir::OpSize::I64}});
}

TEST_CASE("decode CMPSQ via 48 A7 compares qwords [RSI] and [RDI] then advances by 8") {
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0xA7}, r);
    REQUIRE(d.bytes_consumed == 2);
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadMemTSO{0u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op == ir::Op{ir::LoadMemTSO{2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op == ir::Op{ir::CmpFlags{1u, 3u, ir::OpSize::I64}});
    REQUIRE(d.stmts[5].op == ir::Op{ir::Constant{8u, ir::OpSize::I64}});
}

TEST_CASE("decode SCASB via AE compares AL with [RDI] and advances RDI by 1") {
    ir::Ref r = 0;
    auto d = decode_ok({0xAE}, r);
    REQUIRE(d.bytes_consumed == 1);
    REQUIRE(d.stmts.size() == 7);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I8}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rdi, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::LoadMemTSO{1u, ir::OpSize::I8}});
    REQUIRE(d.stmts[3].op == ir::Op{ir::CmpFlags{0u, 2u, ir::OpSize::I8}});
    REQUIRE(d.stmts[4].op == ir::Op{ir::Constant{1u, ir::OpSize::I64}});
}

TEST_CASE("decode SCASQ via 48 AF compares RAX with [RDI] and advances RDI by 8") {
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0xAF}, r);
    REQUIRE(d.bytes_consumed == 2);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::LoadMemTSO{1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op == ir::Op{ir::CmpFlags{0u, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op == ir::Op{ir::Constant{8u, ir::OpSize::I64}});
}

// REP STOSB used to be rejected; F1-DC-066 (commit pending) now
// accepts it as an InlineAsm{bytes} placeholder until the proper
// IR-level loop expansion lands. This regression pin documents the
// new contract.
TEST_CASE("decode REP STOSB (F3 AA) is accepted as InlineAsm placeholder") {
    ir::Ref r = 0;
    auto res = decode_any({0xF3, 0xAA}, r);
    REQUIRE(std::holds_alternative<Decoded>(res));
}

// ---------------------------------------------------------------------
// F2-IR-003 SSE2 integer SIMD decoder
// ---------------------------------------------------------------------

TEST_CASE("decode PADDB xmm0, xmm1 (66 0F FC C1) — Add + B16 lane") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0xFC, 0xC1}, r);
    REQUIRE(d.bytes_consumed == 4);
    // 4 IR stmts: load dst, load src, vbinop, store dst.
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(std::holds_alternative<ir::LoadVecReg>(d.stmts[0].op));
    REQUIRE(std::get<ir::LoadVecReg>(d.stmts[0].op).xmm_index == 0u);
    REQUIRE(std::get<ir::LoadVecReg>(d.stmts[1].op).xmm_index == 1u);
    REQUIRE(std::holds_alternative<ir::VecBinOp>(d.stmts[2].op));
    const auto& vop = std::get<ir::VecBinOp>(d.stmts[2].op);
    REQUIRE(vop.op == ir::VecBinOpKind::Add);
    REQUIRE(vop.lane == ir::VecLane::B16);
    REQUIRE(std::holds_alternative<ir::StoreVecReg>(d.stmts[3].op));
}

TEST_CASE("decode PADDW xmm0, xmm1 (66 0F FD C1) — H8 lane") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0xFD, 0xC1}, r);
    REQUIRE(std::get<ir::VecBinOp>(d.stmts[2].op).lane == ir::VecLane::H8);
}

TEST_CASE("decode PADDD xmm0, xmm1 (66 0F FE C1) — S4 lane") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0xFE, 0xC1}, r);
    REQUIRE(std::get<ir::VecBinOp>(d.stmts[2].op).lane == ir::VecLane::S4);
}

TEST_CASE("decode PADDQ xmm0, xmm1 (66 0F D4 C1) — D2 lane") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0xD4, 0xC1}, r);
    REQUIRE(std::get<ir::VecBinOp>(d.stmts[2].op).lane == ir::VecLane::D2);
}

TEST_CASE("decode PSUBB xmm0, xmm1 (66 0F F8 C1) — Sub + B16") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0xF8, 0xC1}, r);
    const auto& vop = std::get<ir::VecBinOp>(d.stmts[2].op);
    REQUIRE(vop.op == ir::VecBinOpKind::Sub);
    REQUIRE(vop.lane == ir::VecLane::B16);
}

TEST_CASE("decode PXOR xmm0, xmm0 (66 0F EF C0) — idiomatic zero, Xor") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0xEF, 0xC0}, r);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(std::get<ir::LoadVecReg>(d.stmts[0].op).xmm_index == 0u);
    REQUIRE(std::get<ir::LoadVecReg>(d.stmts[1].op).xmm_index == 0u);
    REQUIRE(std::get<ir::VecBinOp>(d.stmts[2].op).op == ir::VecBinOpKind::Xor);
    REQUIRE(std::get<ir::StoreVecReg>(d.stmts[3].op).xmm_index == 0u);
}

TEST_CASE("decode PAND xmm0, xmm1 (66 0F DB C1) — And") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0xDB, 0xC1}, r);
    REQUIRE(std::get<ir::VecBinOp>(d.stmts[2].op).op == ir::VecBinOpKind::And);
}

TEST_CASE("decode POR xmm0, xmm1 (66 0F EB C1) — Or") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0xEB, 0xC1}, r);
    REQUIRE(std::get<ir::VecBinOp>(d.stmts[2].op).op == ir::VecBinOpKind::Or);
}

TEST_CASE("decode PADDB with REX.R/B selects xmm8/xmm15 correctly") {
    ir::Ref r = 0;
    // 41 = REX.B — extends rm to xmm8..15. 44 = REX.R — extends reg.
    // Use 0x45 = REX.R+B → both extended. paddb xmm8, xmm9 → 66 45 0F FC C1
    auto d = decode_ok({0x66, 0x45, 0x0F, 0xFC, 0xC1}, r);
    REQUIRE(d.bytes_consumed == 5);
    REQUIRE(std::get<ir::LoadVecReg>(d.stmts[0].op).xmm_index == 8u);
    REQUIRE(std::get<ir::LoadVecReg>(d.stmts[1].op).xmm_index == 9u);
}

TEST_CASE("decode MOVDQA xmm1, xmm0 (66 0F 6F C8) — load form: dst=reg=xmm1, src=rm=xmm0") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x6F, 0xC8}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 2);
    REQUIRE(std::get<ir::LoadVecReg>(d.stmts[0].op).xmm_index == 0u);
    REQUIRE(std::get<ir::StoreVecReg>(d.stmts[1].op).xmm_index == 1u);
}

TEST_CASE("decode MOVDQA xmm0, xmm1 (66 0F 7F C8) — store form: dst=rm=xmm1, src=reg=xmm0") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x7F, 0xC8}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 2);
    REQUIRE(std::get<ir::LoadVecReg>(d.stmts[0].op).xmm_index == 1u);
    REQUIRE(std::get<ir::StoreVecReg>(d.stmts[1].op).xmm_index == 0u);
}

TEST_CASE("decode MOVDQU xmm1, xmm0 (F3 0F 6F C8) — unaligned variant decodes the same reg-direct") {
    ir::Ref r = 0;
    auto d = decode_ok({0xF3, 0x0F, 0x6F, 0xC8}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 2);
    REQUIRE(std::get<ir::LoadVecReg>(d.stmts[0].op).xmm_index == 0u);
    REQUIRE(std::get<ir::StoreVecReg>(d.stmts[1].op).xmm_index == 1u);
}

TEST_CASE("decode ADDPS xmm0, xmm1 (0F 58 C1) — VecFpBinOp.Add S4") {
    ir::Ref r = 0;
    auto d = decode_ok({0x0F, 0x58, 0xC1}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 4);
    auto vfb = std::get<ir::VecFpBinOp>(d.stmts[2].op);
    REQUIRE(vfb.op   == ir::VecFpBinOpKind::Add);
    REQUIRE(vfb.size == ir::VecFpSize::S4);
}

TEST_CASE("decode MULPD xmm0, xmm1 (66 0F 59 C1) — VecFpBinOp.Mul D2") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x59, 0xC1}, r);
    REQUIRE(d.bytes_consumed == 4);
    auto vfb = std::get<ir::VecFpBinOp>(d.stmts[2].op);
    REQUIRE(vfb.op   == ir::VecFpBinOpKind::Mul);
    REQUIRE(vfb.size == ir::VecFpSize::D2);
}

TEST_CASE("decode SUBPS xmm0, xmm1 (0F 5C C1) — VecFpBinOp.Sub S4") {
    ir::Ref r = 0;
    auto d = decode_ok({0x0F, 0x5C, 0xC1}, r);
    auto vfb = std::get<ir::VecFpBinOp>(d.stmts[2].op);
    REQUIRE(vfb.op   == ir::VecFpBinOpKind::Sub);
    REQUIRE(vfb.size == ir::VecFpSize::S4);
}

TEST_CASE("decode DIVPD xmm0, xmm1 (66 0F 5E C1) — VecFpBinOp.Div D2") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x5E, 0xC1}, r);
    auto vfb = std::get<ir::VecFpBinOp>(d.stmts[2].op);
    REQUIRE(vfb.op   == ir::VecFpBinOpKind::Div);
    REQUIRE(vfb.size == ir::VecFpSize::D2);
}

TEST_CASE("decode ADDSS xmm0, xmm1 (F3 0F 58 C1) — scalar Add F32") {
    ir::Ref r = 0;
    auto d = decode_ok({0xF3, 0x0F, 0x58, 0xC1}, r);
    REQUIRE(d.bytes_consumed == 4);
    auto vfb = std::get<ir::VecFpScalarBinOp>(d.stmts[2].op);
    REQUIRE(vfb.op   == ir::VecFpBinOpKind::Add);
    REQUIRE(vfb.size == ir::FpSize::F32);
}

TEST_CASE("decode MULSD xmm0, xmm1 (F2 0F 59 C1) — scalar Mul F64") {
    ir::Ref r = 0;
    auto d = decode_ok({0xF2, 0x0F, 0x59, 0xC1}, r);
    auto vfb = std::get<ir::VecFpScalarBinOp>(d.stmts[2].op);
    REQUIRE(vfb.op   == ir::VecFpBinOpKind::Mul);
    REQUIRE(vfb.size == ir::FpSize::F64);
}

TEST_CASE("decode DIVSS xmm0, xmm1 (F3 0F 5E C1) — scalar Div F32") {
    ir::Ref r = 0;
    auto d = decode_ok({0xF3, 0x0F, 0x5E, 0xC1}, r);
    auto vfb = std::get<ir::VecFpScalarBinOp>(d.stmts[2].op);
    REQUIRE(vfb.op   == ir::VecFpBinOpKind::Div);
    REQUIRE(vfb.size == ir::FpSize::F32);
}

TEST_CASE("decode MOVAPS xmm0, xmm1 (0F 28 C1) — F2-IR-013 reg-reg load") {
    ir::Ref r = 0;
    auto d = decode_ok({0x0F, 0x28, 0xC1}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(std::holds_alternative<ir::LoadVecReg>(d.stmts[0].op));
    REQUIRE(std::holds_alternative<ir::StoreVecReg>(d.stmts[1].op));
}

TEST_CASE("decode MOVUPS [rcx], xmm0 (0F 11 01) — F2-IR-013 unaligned store") {
    ir::Ref r = 0;
    auto d = decode_ok({0x0F, 0x11, 0x01}, r);
    bool saw_storevec = false;
    for (const auto& st : d.stmts) {
        if (std::holds_alternative<ir::StoreVec>(st.op)) saw_storevec = true;
    }
    REQUIRE(saw_storevec);
}

TEST_CASE("decode MOVAPD xmm0, [rcx] (66 0F 28 01) — F2-IR-013 prefixed load") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x28, 0x01}, r);
    bool saw_loadvec = false;
    for (const auto& st : d.stmts) {
        if (std::holds_alternative<ir::LoadVec>(st.op)) saw_loadvec = true;
    }
    REQUIRE(saw_loadvec);
}

TEST_CASE("decode PMULLW xmm0, xmm1 (66 0F D5 C1) — VecBinOp.Mul H8") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0xD5, 0xC1}, r);
    auto vb = std::get<ir::VecBinOp>(d.stmts[2].op);
    REQUIRE(vb.op   == ir::VecBinOpKind::Mul);
    REQUIRE(vb.lane == ir::VecLane::H8);
}

TEST_CASE("decode UNPCKLPS xmm0, xmm1 (0F 14 C1) — F2-IR-015 FP unpack low S4") {
    ir::Ref r = 0;
    auto d = decode_ok({0x0F, 0x14, 0xC1}, r);
    auto vu = std::get<ir::VecUnpack>(d.stmts[2].op);
    REQUIRE(vu.is_high == false);
    REQUIRE(vu.lane    == ir::VecLane::S4);
}

TEST_CASE("decode UNPCKHPD xmm0, xmm1 (66 0F 15 C1) — FP unpack high D2") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x15, 0xC1}, r);
    auto vu = std::get<ir::VecUnpack>(d.stmts[2].op);
    REQUIRE(vu.is_high == true);
    REQUIRE(vu.lane    == ir::VecLane::D2);
}

TEST_CASE("decode PUNPCKLBW xmm0, xmm1 (66 0F 60 C1) — VecUnpack low B16") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x60, 0xC1}, r);
    auto vu = std::get<ir::VecUnpack>(d.stmts[2].op);
    REQUIRE(vu.is_high == false);
    REQUIRE(vu.lane    == ir::VecLane::B16);
}

TEST_CASE("decode PUNPCKHQDQ xmm0, xmm1 (66 0F 6D C1) — VecUnpack high D2") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x6D, 0xC1}, r);
    auto vu = std::get<ir::VecUnpack>(d.stmts[2].op);
    REQUIRE(vu.is_high == true);
    REQUIRE(vu.lane    == ir::VecLane::D2);
}

TEST_CASE("decode PSLLDQ xmm0, 4 (66 0F 73 F8 04) — VecShiftBytes left") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x73, 0xF8, 0x04}, r);
    auto vsb = std::get<ir::VecShiftBytes>(d.stmts[1].op);
    REQUIRE(vsb.is_left == true);
    REQUIRE(vsb.count   == 4u);
}

TEST_CASE("decode PSRLDQ xmm0, 8 (66 0F 73 D8 08) — VecShiftBytes right") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x73, 0xD8, 0x08}, r);
    auto vsb = std::get<ir::VecShiftBytes>(d.stmts[1].op);
    REQUIRE(vsb.is_left == false);
    REQUIRE(vsb.count   == 8u);
}

TEST_CASE("decode PSLLD xmm0, 4 (66 0F 72 F0 04) — VecShiftImm.ShiftL S4") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x72, 0xF0, 0x04}, r);
    REQUIRE(d.bytes_consumed == 5);
    auto vs = std::get<ir::VecShiftImm>(d.stmts[1].op);
    REQUIRE(vs.kind  == ir::VecShiftKind::ShiftL);
    REQUIRE(vs.lane  == ir::VecLane::S4);
    REQUIRE(vs.count == 4u);
}

TEST_CASE("decode PSRAW xmm0, 1 (66 0F 71 E0 01) — VecShiftImm.ArithShr H8") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x71, 0xE0, 0x01}, r);
    auto vs = std::get<ir::VecShiftImm>(d.stmts[1].op);
    REQUIRE(vs.kind  == ir::VecShiftKind::ArithShr);
    REQUIRE(vs.lane  == ir::VecLane::H8);
}

TEST_CASE("decode PSHUFD xmm0, xmm1, 0x1B (66 0F 70 C1 1B) — VecShuffle32x4 reverse") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x70, 0xC1, 0x1B}, r);
    REQUIRE(d.bytes_consumed == 5);
    auto vs = std::get<ir::VecShuffle32x4>(d.stmts[1].op);
    REQUIRE(vs.control == 0x1B);
}

TEST_CASE("decode PCMPEQB xmm0, xmm1 (66 0F 74 C1) — VecCmp.Eq B16") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x74, 0xC1}, r);
    auto vc = std::get<ir::VecCmp>(d.stmts[2].op);
    REQUIRE(vc.kind == ir::VecCmpKind::Eq);
    REQUIRE(vc.lane == ir::VecLane::B16);
}

TEST_CASE("decode PCMPGTD xmm0, xmm1 (66 0F 66 C1) — VecCmp.Gt S4") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x66, 0xC1}, r);
    auto vc = std::get<ir::VecCmp>(d.stmts[2].op);
    REQUIRE(vc.kind == ir::VecCmpKind::Gt);
    REQUIRE(vc.lane == ir::VecLane::S4);
}

TEST_CASE("decode MOVMSKPS eax, xmm0 (0F 50 C0) — F2-IR-029 FP sign mask S4") {
    ir::Ref r = 0;
    auto d = decode_ok({0x0F, 0x50, 0xC0}, r);
    auto vm = std::get<ir::VecMaskFp>(d.stmts[1].op);
    REQUIRE(vm.is_pd == false);
}

TEST_CASE("decode MOVMSKPD eax, xmm0 (66 0F 50 C0) — D2 form") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x50, 0xC0}, r);
    auto vm = std::get<ir::VecMaskFp>(d.stmts[1].op);
    REQUIRE(vm.is_pd == true);
}

TEST_CASE("decode PSHUFLW xmm0, xmm1, 0x1B (F2 0F 70 C1 1B) — F2-IR-028 low-half H8 shuffle") {
    ir::Ref r = 0;
    auto d = decode_ok({0xF2, 0x0F, 0x70, 0xC1, 0x1B}, r);
    auto vs = std::get<ir::VecShuffleH4>(d.stmts[1].op);
    REQUIRE(vs.is_high == false);
    REQUIRE(vs.control == 0x1B);
}

TEST_CASE("decode PSHUFHW xmm0, xmm1, 0xE4 (F3 0F 70 C1 E4) — high-half H8 shuffle") {
    ir::Ref r = 0;
    auto d = decode_ok({0xF3, 0x0F, 0x70, 0xC1, 0xE4}, r);
    auto vs = std::get<ir::VecShuffleH4>(d.stmts[1].op);
    REQUIRE(vs.is_high == true);
}

TEST_CASE("decode UCOMISS xmm0, xmm1 (0F 2E C1) — F2-IR-026 FP compare flags") {
    ir::Ref r = 0;
    auto d = decode_ok({0x0F, 0x2E, 0xC1}, r);
    auto wf = std::get<ir::WriteFlagsFp>(d.stmts[2].op);
    REQUIRE(wf.size == ir::FpSize::F32);
}

TEST_CASE("decode UCOMISD xmm0, xmm1 (66 0F 2E C1) — F64 form") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x2E, 0xC1}, r);
    auto wf = std::get<ir::WriteFlagsFp>(d.stmts[2].op);
    REQUIRE(wf.size == ir::FpSize::F64);
}

TEST_CASE("decode PMOVMSKB eax, xmm0 (66 0F D7 C0) — F2-IR-027") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0xD7, 0xC0}, r);
    REQUIRE(std::holds_alternative<ir::VecMaskMsb>(d.stmts[1].op));
}

TEST_CASE("decode ROUNDPS xmm0, xmm1, 1 (66 0F 3A 08 C1 01) — F2-IR-042 round-down packed S4") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x3A, 0x08, 0xC1, 0x01}, r);
    auto vr = std::get<ir::VecFpRound>(d.stmts[2].op);
    REQUIRE(vr.size      == ir::FpSize::F32);
    REQUIRE(vr.is_packed == true);
    REQUIRE(vr.mode      == 1);
}

TEST_CASE("decode ROUNDSD xmm0, xmm1, 3 (66 0F 3A 0B C1 03) — scalar truncate F64") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x3A, 0x0B, 0xC1, 0x03}, r);
    auto vr = std::get<ir::VecFpRound>(d.stmts[2].op);
    REQUIRE(vr.size      == ir::FpSize::F64);
    REQUIRE(vr.is_packed == false);
    REQUIRE(vr.mode      == 3);
}

TEST_CASE("decode PTEST xmm0, xmm1 (66 0F 38 17 C1) — F2-IR-047 SSE4.1 bitwise test") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x38, 0x17, 0xC1}, r);
    REQUIRE(std::holds_alternative<ir::WriteFlagsPtest>(d.stmts[2].op));
}

TEST_CASE("decode PBLENDVB xmm0, xmm1 (66 0F 38 10 C1) — F2-IR-046 byte blend") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x38, 0x10, 0xC1}, r);
    auto vb = std::get<ir::VecBlend>(d.stmts[3].op);
    REQUIRE(vb.lane == ir::VecLane::B16);
}

TEST_CASE("decode BLENDVPS xmm0, xmm1 (66 0F 38 14 C1) — float blend") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x38, 0x14, 0xC1}, r);
    auto vb = std::get<ir::VecBlend>(d.stmts[3].op);
    REQUIRE(vb.lane == ir::VecLane::S4);
}

TEST_CASE("decode PMOVZXBW xmm0, xmm1 (66 0F 38 30 C1) — F2-IR-041 zero-ext B→H") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x38, 0x30, 0xC1}, r);
    auto ve = std::get<ir::VecExtend>(d.stmts[1].op);
    REQUIRE(ve.is_signed   == false);
    REQUIRE(ve.narrow_lane == ir::VecLane::B16);
    REQUIRE(ve.wide_lane   == ir::VecLane::H8);
}

TEST_CASE("decode PMOVSXBQ xmm0, xmm1 (66 0F 38 22 C1) — sign-ext B→Q (3-step)") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x38, 0x22, 0xC1}, r);
    auto ve = std::get<ir::VecExtend>(d.stmts[1].op);
    REQUIRE(ve.is_signed   == true);
    REQUIRE(ve.narrow_lane == ir::VecLane::B16);
    REQUIRE(ve.wide_lane   == ir::VecLane::D2);
}

TEST_CASE("decode PMOVZXDQ xmm0, xmm1 (66 0F 38 35 C1) — zero-ext S4→D2") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x38, 0x35, 0xC1}, r);
    auto ve = std::get<ir::VecExtend>(d.stmts[1].op);
    REQUIRE(ve.is_signed   == false);
    REQUIRE(ve.narrow_lane == ir::VecLane::S4);
    REQUIRE(ve.wide_lane   == ir::VecLane::D2);
}

TEST_CASE("decode PEXTRD eax, xmm0, 2 (66 0F 3A 16 C0 02) — SSE4.1 dword extract S4") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x3A, 0x16, 0xC0, 0x02}, r);
    auto ve = std::get<ir::VecExtractLaneU>(d.stmts[1].op);
    REQUIRE(ve.lane     == ir::VecLane::S4);
    REQUIRE(ve.lane_idx == 2);
}

TEST_CASE("decode PEXTRQ rax, xmm0, 1 (66 48 0F 3A 16 C0 01) — qword extract D2 via REX.W") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x48, 0x0F, 0x3A, 0x16, 0xC0, 0x01}, r);
    auto ve = std::get<ir::VecExtractLaneU>(d.stmts[1].op);
    REQUIRE(ve.lane     == ir::VecLane::D2);
    REQUIRE(ve.lane_idx == 1);
}

TEST_CASE("decode PINSRD xmm0, eax, 1 (66 0F 3A 22 C0 01) — dword insert") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x3A, 0x22, 0xC0, 0x01}, r);
    auto vi = std::get<ir::VecInsertLane>(d.stmts[2].op);
    REQUIRE(vi.lane     == ir::VecLane::S4);
    REQUIRE(vi.lane_idx == 1);
}

TEST_CASE("decode PEXTRB eax, xmm0, 5 (66 0F 3A 14 C0 05) — SSE4.1 byte extract") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x3A, 0x14, 0xC0, 0x05}, r);
    auto ve = std::get<ir::VecExtractLaneU>(d.stmts[1].op);
    REQUIRE(ve.lane     == ir::VecLane::B16);
    REQUIRE(ve.lane_idx == 5);
}

TEST_CASE("decode PINSRB xmm0, eax, 7 (66 0F 3A 20 C0 07) — SSE4.1 byte insert") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x3A, 0x20, 0xC0, 0x07}, r);
    auto vi = std::get<ir::VecInsertLane>(d.stmts[2].op);
    REQUIRE(vi.lane     == ir::VecLane::B16);
    REQUIRE(vi.lane_idx == 7);
}

TEST_CASE("decode PALIGNR xmm0, xmm1, 4 (66 0F 3A 0F C1 04) — F2-IR-038 byte concat-shift") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x3A, 0x0F, 0xC1, 0x04}, r);
    auto va = std::get<ir::VecAlignr>(d.stmts[2].op);
    REQUIRE(va.count == 4);
}

TEST_CASE("decode LZCNT eax, ecx (F3 0F BD C1) — F2-IR-045 BMI1 leading-zero count") {
    ir::Ref r = 0;
    auto d = decode_ok({0xF3, 0x0F, 0xBD, 0xC1}, r);
    auto lz = std::get<ir::Lzcnt>(d.stmts[1].op);
    REQUIRE(lz.size == ir::OpSize::I32);
}

TEST_CASE("decode TZCNT rax, rcx (F3 48 0F BC C1) — BMI1 trailing-zero I64") {
    ir::Ref r = 0;
    auto d = decode_ok({0xF3, 0x48, 0x0F, 0xBC, 0xC1}, r);
    auto tz = std::get<ir::Tzcnt>(d.stmts[1].op);
    REQUIRE(tz.size == ir::OpSize::I64);
}

TEST_CASE("decode POPCNT eax, ecx (F3 0F B8 C1) — F2-IR-044 SSE4.2 popcount I32") {
    ir::Ref r = 0;
    auto d = decode_ok({0xF3, 0x0F, 0xB8, 0xC1}, r);
    auto pc = std::get<ir::Popcnt>(d.stmts[1].op);
    REQUIRE(pc.size == ir::OpSize::I32);
}

TEST_CASE("decode POPCNT rax, rcx (F3 48 0F B8 C1) — popcount I64") {
    ir::Ref r = 0;
    auto d = decode_ok({0xF3, 0x48, 0x0F, 0xB8, 0xC1}, r);
    auto pc = std::get<ir::Popcnt>(d.stmts[1].op);
    REQUIRE(pc.size == ir::OpSize::I64);
}

TEST_CASE("decode PCMPGTQ xmm0, xmm1 (66 0F 38 37 C1) — F2-IR-043 SSE4.2 D2 signed >") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x38, 0x37, 0xC1}, r);
    auto vc = std::get<ir::VecCmp>(d.stmts[2].op);
    REQUIRE(vc.kind == ir::VecCmpKind::Gt);
    REQUIRE(vc.lane == ir::VecLane::D2);
}

TEST_CASE("decode PMINSB xmm0, xmm1 (66 0F 38 38 C1) — SSE4.1 signed min B16") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x38, 0x38, 0xC1}, r);
    auto vb = std::get<ir::VecBinOp>(d.stmts[2].op);
    REQUIRE(vb.op   == ir::VecBinOpKind::SMin);
    REQUIRE(vb.lane == ir::VecLane::B16);
}

TEST_CASE("decode PMAXUD xmm0, xmm1 (66 0F 38 3F C1) — SSE4.1 unsigned max S4") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x38, 0x3F, 0xC1}, r);
    auto vb = std::get<ir::VecBinOp>(d.stmts[2].op);
    REQUIRE(vb.op   == ir::VecBinOpKind::UMax);
    REQUIRE(vb.lane == ir::VecLane::S4);
}

TEST_CASE("decode PHADDD xmm0, xmm1 (66 0F 38 02 C1) — F2-IR-037 pairwise add S4") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x38, 0x02, 0xC1}, r);
    auto vb = std::get<ir::VecBinOp>(d.stmts[2].op);
    REQUIRE(vb.op   == ir::VecBinOpKind::PairAddInt);
    REQUIRE(vb.lane == ir::VecLane::S4);
}

TEST_CASE("decode PHSUBW xmm0, xmm1 (66 0F 38 05 C1) — pairwise sub H8") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x38, 0x05, 0xC1}, r);
    auto vb = std::get<ir::VecBinOp>(d.stmts[2].op);
    REQUIRE(vb.op   == ir::VecBinOpKind::PairSubInt);
    REQUIRE(vb.lane == ir::VecLane::H8);
}

TEST_CASE("decode PMULLD xmm0, xmm1 (66 0F 38 40 C1) — SSE4.1 packed S4 multiply") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x38, 0x40, 0xC1}, r);
    auto vb = std::get<ir::VecBinOp>(d.stmts[2].op);
    REQUIRE(vb.op   == ir::VecBinOpKind::Mul);
    REQUIRE(vb.lane == ir::VecLane::S4);
}

TEST_CASE("decode PCMPEQQ xmm0, xmm1 (66 0F 38 29 C1) — SSE4.1 D2 equality") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x38, 0x29, 0xC1}, r);
    auto vc = std::get<ir::VecCmp>(d.stmts[2].op);
    REQUIRE(vc.kind == ir::VecCmpKind::Eq);
    REQUIRE(vc.lane == ir::VecLane::D2);
}

TEST_CASE("decode PSHUFB xmm0, xmm1 (66 0F 38 00 C1) — F2-IR-036 SSSE3 byte shuffle") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x38, 0x00, 0xC1}, r);
    REQUIRE(std::holds_alternative<ir::VecPshufb>(d.stmts[2].op));
}

TEST_CASE("decode PABSB xmm0, xmm1 (66 0F 38 1C C1) — abs B16") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x38, 0x1C, 0xC1}, r);
    auto va = std::get<ir::VecAbs>(d.stmts[1].op);
    REQUIRE(va.lane == ir::VecLane::B16);
}

TEST_CASE("decode PABSD xmm0, xmm1 (66 0F 38 1E C1) — abs S4") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x38, 0x1E, 0xC1}, r);
    auto va = std::get<ir::VecAbs>(d.stmts[1].op);
    REQUIRE(va.lane == ir::VecLane::S4);
}

TEST_CASE("decode LDDQU xmm0, [rcx] (F2 0F F0 01) — F2-IR-035 SSE3 unaligned load") {
    ir::Ref r = 0;
    auto d = decode_ok({0xF2, 0x0F, 0xF0, 0x01}, r);
    bool saw_loadvec = false;
    for (const auto& st : d.stmts) {
        if (std::holds_alternative<ir::LoadVec>(st.op)) saw_loadvec = true;
    }
    REQUIRE(saw_loadvec);
}

TEST_CASE("decode MOVNTDQ [rcx], xmm0 (66 0F E7 01) — non-temporal store alias") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0xE7, 0x01}, r);
    bool saw_storevec = false;
    for (const auto& st : d.stmts) {
        if (std::holds_alternative<ir::StoreVec>(st.op)) saw_storevec = true;
    }
    REQUIRE(saw_storevec);
}

TEST_CASE("decode MOVNTPS [rcx], xmm0 (0F 2B 01) — non-temporal FP store") {
    ir::Ref r = 0;
    auto d = decode_ok({0x0F, 0x2B, 0x01}, r);
    bool saw_storevec = false;
    for (const auto& st : d.stmts) {
        if (std::holds_alternative<ir::StoreVec>(st.op)) saw_storevec = true;
    }
    REQUIRE(saw_storevec);
}

TEST_CASE("decode CMPEQPS xmm0, xmm1 (0F C2 C1 00) — F2-IR-034 packed eq F32") {
    ir::Ref r = 0;
    auto d = decode_ok({0x0F, 0xC2, 0xC1, 0x00}, r);
    auto vc = std::get<ir::VecFpCompare>(d.stmts[2].op);
    REQUIRE(vc.pred == ir::VecFpCmpPred::Eq);
    REQUIRE(vc.size == ir::FpSize::F32);
    REQUIRE(vc.is_packed == true);
}

TEST_CASE("decode CMPLTSS xmm0, xmm1 (F3 0F C2 C1 01) — scalar lt F32") {
    ir::Ref r = 0;
    auto d = decode_ok({0xF3, 0x0F, 0xC2, 0xC1, 0x01}, r);
    auto vc = std::get<ir::VecFpCompare>(d.stmts[2].op);
    REQUIRE(vc.pred == ir::VecFpCmpPred::Lt);
    REQUIRE(vc.is_packed == false);
}

TEST_CASE("decode CMPNLEPD xmm0, xmm1 (66 0F C2 C1 06) — packed nle F64") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0xC2, 0xC1, 0x06}, r);
    auto vc = std::get<ir::VecFpCompare>(d.stmts[2].op);
    REQUIRE(vc.pred == ir::VecFpCmpPred::Nle);
    REQUIRE(vc.size == ir::FpSize::F64);
}

TEST_CASE("decode HADDPS xmm0, xmm1 (F2 0F 7C C1) — F2-IR-032 horizontal-add S4") {
    ir::Ref r = 0;
    auto d = decode_ok({0xF2, 0x0F, 0x7C, 0xC1}, r);
    auto vfb = std::get<ir::VecFpBinOp>(d.stmts[2].op);
    REQUIRE(vfb.op   == ir::VecFpBinOpKind::HAdd);
    REQUIRE(vfb.size == ir::VecFpSize::S4);
}

TEST_CASE("decode HADDPD xmm0, xmm1 (66 0F 7C C1) — D2 form") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x7C, 0xC1}, r);
    auto vfb = std::get<ir::VecFpBinOp>(d.stmts[2].op);
    REQUIRE(vfb.size == ir::VecFpSize::D2);
}

TEST_CASE("decode PSADBW xmm0, xmm1 (66 0F F6 C1) — F2-IR-031 sum-abs-diff bytes") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0xF6, 0xC1}, r);
    auto vb = std::get<ir::VecBinOp>(d.stmts[2].op);
    REQUIRE(vb.op == ir::VecBinOpKind::SadBw);
}

TEST_CASE("decode PMULUDQ xmm0, xmm1 (66 0F F4 C1) — F2-IR-030 u32→u64 multiply") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0xF4, 0xC1}, r);
    auto vb = std::get<ir::VecBinOp>(d.stmts[2].op);
    REQUIRE(vb.op == ir::VecBinOpKind::UMul32To64);
}

TEST_CASE("decode PMULHW xmm0, xmm1 (66 0F E5 C1) — F2-IR-025 signed mul-high H8") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0xE5, 0xC1}, r);
    auto vb = std::get<ir::VecBinOp>(d.stmts[2].op);
    REQUIRE(vb.op   == ir::VecBinOpKind::SMulHi);
    REQUIRE(vb.lane == ir::VecLane::H8);
}

TEST_CASE("decode PMULHUW xmm0, xmm1 (66 0F E4 C1) — unsigned mul-high H8") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0xE4, 0xC1}, r);
    auto vb = std::get<ir::VecBinOp>(d.stmts[2].op);
    REQUIRE(vb.op == ir::VecBinOpKind::UMulHi);
}

TEST_CASE("decode PMINUB xmm0, xmm1 (66 0F DA C1) — F2-IR-024 unsigned min B16") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0xDA, 0xC1}, r);
    auto vb = std::get<ir::VecBinOp>(d.stmts[2].op);
    REQUIRE(vb.op   == ir::VecBinOpKind::UMin);
    REQUIRE(vb.lane == ir::VecLane::B16);
}

TEST_CASE("decode PMAXSW xmm0, xmm1 (66 0F EE C1) — signed max H8") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0xEE, 0xC1}, r);
    auto vb = std::get<ir::VecBinOp>(d.stmts[2].op);
    REQUIRE(vb.op   == ir::VecBinOpKind::SMax);
    REQUIRE(vb.lane == ir::VecLane::H8);
}

TEST_CASE("decode PADDSB xmm0, xmm1 (66 0F EC C1) — F2-IR-023 signed sat add B16") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0xEC, 0xC1}, r);
    auto vb = std::get<ir::VecBinOp>(d.stmts[2].op);
    REQUIRE(vb.op   == ir::VecBinOpKind::SqAdd);
    REQUIRE(vb.lane == ir::VecLane::B16);
}

TEST_CASE("decode PADDUSW xmm0, xmm1 (66 0F DD C1) — unsigned sat add H8") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0xDD, 0xC1}, r);
    auto vb = std::get<ir::VecBinOp>(d.stmts[2].op);
    REQUIRE(vb.op   == ir::VecBinOpKind::UqAdd);
    REQUIRE(vb.lane == ir::VecLane::H8);
}

TEST_CASE("decode PSUBUSB xmm0, xmm1 (66 0F D8 C1) — unsigned sat sub B16") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0xD8, 0xC1}, r);
    auto vb = std::get<ir::VecBinOp>(d.stmts[2].op);
    REQUIRE(vb.op == ir::VecBinOpKind::UqSub);
}

TEST_CASE("decode PINSRW xmm0, eax, 3 (66 0F C4 C0 03) — F2-IR-022 insert word") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0xC4, 0xC0, 0x03}, r);
    auto vi = std::get<ir::VecInsertLane>(d.stmts[2].op);
    REQUIRE(vi.lane_idx == 3);
    REQUIRE(vi.lane     == ir::VecLane::H8);
}

TEST_CASE("decode PEXTRW eax, xmm0, 5 (66 0F C5 C0 05) — extract word") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0xC5, 0xC0, 0x05}, r);
    auto ve = std::get<ir::VecExtractLaneU>(d.stmts[1].op);
    REQUIRE(ve.lane_idx == 5);
    REQUIRE(ve.lane     == ir::VecLane::H8);
}

TEST_CASE("decode MOVSS xmm0, xmm1 (F3 0F 10 C1) — F2-IR-021 reg-reg upper-preserve") {
    ir::Ref r = 0;
    auto d = decode_ok({0xF3, 0x0F, 0x10, 0xC1}, r);
    auto cv = std::get<ir::FpCvtScalar>(d.stmts[2].op);
    REQUIRE(cv.src_size == ir::FpSize::F32);
    REQUIRE(cv.dst_size == ir::FpSize::F32);
}

TEST_CASE("decode MOVSD xmm0, [rcx] (F2 0F 10 01) — mem load zeros upper") {
    ir::Ref r = 0;
    auto d = decode_ok({0xF2, 0x0F, 0x10, 0x01}, r);
    bool saw_xmm_from_gpr = false;
    for (const auto& st : d.stmts) {
        if (std::holds_alternative<ir::XmmFromGpr>(st.op)) saw_xmm_from_gpr = true;
    }
    REQUIRE(saw_xmm_from_gpr);
}

TEST_CASE("decode MOVSS [rcx], xmm0 (F3 0F 11 01) — store low 32 bits") {
    ir::Ref r = 0;
    auto d = decode_ok({0xF3, 0x0F, 0x11, 0x01}, r);
    bool saw_gpr_from_xmm = false;
    bool saw_storemem = false;
    for (const auto& st : d.stmts) {
        if (std::holds_alternative<ir::GprFromXmm>(st.op)) saw_gpr_from_xmm = true;
        if (std::holds_alternative<ir::StoreMemTSO>(st.op)) saw_storemem = true;
    }
    REQUIRE(saw_gpr_from_xmm);
    REQUIRE(saw_storemem);
}

TEST_CASE("decode SHUFPS xmm0, xmm1, 0xE4 (0F C6 C1 E4) — F2-IR-020 FP shuffle") {
    ir::Ref r = 0;
    auto d = decode_ok({0x0F, 0xC6, 0xC1, 0xE4}, r);
    auto vs = std::get<ir::VecShuffle2Src>(d.stmts[2].op);
    REQUIRE(vs.is_pd   == false);
    REQUIRE(vs.control == 0xE4);
}

TEST_CASE("decode SHUFPD xmm0, xmm1, 0x01 (66 0F C6 C1 01) — D2 form") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0xC6, 0xC1, 0x01}, r);
    auto vs = std::get<ir::VecShuffle2Src>(d.stmts[2].op);
    REQUIRE(vs.is_pd == true);
}

TEST_CASE("decode XORPS xmm0, xmm0 (0F 57 C0) — F2-IR-018 bitwise FP zero") {
    ir::Ref r = 0;
    auto d = decode_ok({0x0F, 0x57, 0xC0}, r);
    auto vb = std::get<ir::VecBinOp>(d.stmts[2].op);
    REQUIRE(vb.op == ir::VecBinOpKind::Xor);
}

TEST_CASE("decode ANDPD xmm0, xmm1 (66 0F 54 C1) — bitwise FP and") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x54, 0xC1}, r);
    auto vb = std::get<ir::VecBinOp>(d.stmts[2].op);
    REQUIRE(vb.op == ir::VecBinOpKind::And);
}

TEST_CASE("decode CVTSS2SD xmm0, xmm1 (F3 0F 5A C1) — F2-IR-017 single→double") {
    ir::Ref r = 0;
    auto d = decode_ok({0xF3, 0x0F, 0x5A, 0xC1}, r);
    auto cv = std::get<ir::FpCvtScalar>(d.stmts[2].op);
    REQUIRE(cv.src_size == ir::FpSize::F32);
    REQUIRE(cv.dst_size == ir::FpSize::F64);
}

TEST_CASE("decode CVTSD2SS xmm0, xmm1 (F2 0F 5A C1) — double→single") {
    ir::Ref r = 0;
    auto d = decode_ok({0xF2, 0x0F, 0x5A, 0xC1}, r);
    auto cv = std::get<ir::FpCvtScalar>(d.stmts[2].op);
    REQUIRE(cv.src_size == ir::FpSize::F64);
    REQUIRE(cv.dst_size == ir::FpSize::F32);
}

TEST_CASE("decode CVTSI2SS xmm0, eax (F3 0F 2A C0) — F2-IR-016 int→FP F32") {
    ir::Ref r = 0;
    auto d = decode_ok({0xF3, 0x0F, 0x2A, 0xC0}, r);
    auto cv = std::get<ir::IntToFpScalar>(d.stmts[1].op);
    REQUIRE(cv.fp_size  == ir::FpSize::F32);
    REQUIRE(cv.int_size == ir::OpSize::I32);
}

TEST_CASE("decode CVTSI2SD xmm0, rax (F2 48 0F 2A C0) — int64→FP F64 via REX.W") {
    ir::Ref r = 0;
    auto d = decode_ok({0xF2, 0x48, 0x0F, 0x2A, 0xC0}, r);
    auto cv = std::get<ir::IntToFpScalar>(d.stmts[1].op);
    REQUIRE(cv.fp_size  == ir::FpSize::F64);
    REQUIRE(cv.int_size == ir::OpSize::I64);
}

TEST_CASE("decode CVTTSS2SI eax, xmm0 (F3 0F 2C C0) — FP→int truncate I32") {
    ir::Ref r = 0;
    auto d = decode_ok({0xF3, 0x0F, 0x2C, 0xC0}, r);
    auto cv = std::get<ir::FpToIntScalar>(d.stmts[1].op);
    REQUIRE(cv.fp_size  == ir::FpSize::F32);
    REQUIRE(cv.int_size == ir::OpSize::I32);
}

TEST_CASE("decode MOVD xmm0, eax (66 0F 6E C0) — XmmFromGpr.I32") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x6E, 0xC0}, r);
    REQUIRE(d.bytes_consumed == 4);
    auto x = std::get<ir::XmmFromGpr>(d.stmts[1].op);
    REQUIRE(x.size == ir::OpSize::I32);
}

TEST_CASE("decode MOVQ xmm0, rax (66 48 0F 6E C0) — XmmFromGpr.I64 via REX.W") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x48, 0x0F, 0x6E, 0xC0}, r);
    REQUIRE(d.bytes_consumed == 5);
    auto x = std::get<ir::XmmFromGpr>(d.stmts[1].op);
    REQUIRE(x.size == ir::OpSize::I64);
}

TEST_CASE("decode MOVD eax, xmm0 (66 0F 7E C0) — GprFromXmm.I32") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x7E, 0xC0}, r);
    REQUIRE(d.bytes_consumed == 4);
    auto x = std::get<ir::GprFromXmm>(d.stmts[1].op);
    REQUIRE(x.size == ir::OpSize::I32);
}

TEST_CASE("decode MOVQ rax, xmm0 (66 48 0F 7E C0) — GprFromXmm.I64") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x48, 0x0F, 0x7E, 0xC0}, r);
    auto x = std::get<ir::GprFromXmm>(d.stmts[1].op);
    REQUIRE(x.size == ir::OpSize::I64);
}

TEST_CASE("decode ADDPS xmm0, [rcx] (0F 58 01) — F2-IR-007 memory form via LoadVec") {
    ir::Ref r = 0;
    auto d = decode_ok({0x0F, 0x58, 0x01}, r);
    REQUIRE(d.bytes_consumed == 3);
    bool saw_loadvec = false;
    for (const auto& st : d.stmts) {
        if (std::holds_alternative<ir::LoadVec>(st.op)) saw_loadvec = true;
    }
    REQUIRE(saw_loadvec);
}

TEST_CASE("decode MOVDQA xmm0, [rcx] (66 0F 6F 01) — memory load form") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x6F, 0x01}, r);
    bool saw_loadvec = false;
    for (const auto& st : d.stmts) {
        if (std::holds_alternative<ir::LoadVec>(st.op)) saw_loadvec = true;
    }
    REQUIRE(saw_loadvec);
}

TEST_CASE("decode MOVDQA [rcx], xmm0 (66 0F 7F 01) — memory store form") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0x7F, 0x01}, r);
    bool saw_storevec = false;
    for (const auto& st : d.stmts) {
        if (std::holds_alternative<ir::StoreVec>(st.op)) saw_storevec = true;
    }
    REQUIRE(saw_storevec);
}

TEST_CASE("decode PADDB xmm0, [rcx] (66 0F FC 01) — memory operand via LoadVec") {
    ir::Ref r = 0;
    auto d = decode_ok({0x66, 0x0F, 0xFC, 0x01}, r);
    bool saw_loadvec = false;
    for (const auto& st : d.stmts) {
        if (std::holds_alternative<ir::LoadVec>(st.op)) saw_loadvec = true;
    }
    REQUIRE(saw_loadvec);
}

TEST_CASE("Error: HLT (F4) is rejected as UnsupportedEncoding") {
    ir::Ref r = 0;
    auto res = decode_any({0xF4}, r);
    REQUIRE(std::holds_alternative<DecodeError>(res));
    REQUIRE(std::get<DecodeError>(res) == DecodeError::UnsupportedEncoding);
}

TEST_CASE("Error: memory form MOV with SIB byte (rm=100) needs a SIB byte") {
    // 48 89 04 ... now requires a SIB byte (Codex's SIB support).
    // Three bytes alone is truncated: ModR/M says SIB follows, but no
    // SIB byte is present → TruncatedInput.
    ir::Ref r = 0;
    auto res = decode_any({0x48, 0x89, 0x04}, r);
    REQUIRE(std::holds_alternative<DecodeError>(res));
    REQUIRE(std::get<DecodeError>(res) == DecodeError::TruncatedInput);
}

TEST_CASE("decode mod=00 rm=101 as RIP-relative (48 89 05 disp32)") {
    // 48 89 05 XX XX XX XX  (MOV [rip+disp32], rax). Codex's RIP-rel
    // support computes the absolute address from rip_after + disp32
    // at decode time, collapsing it into a Constant operand.
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x89, 0x05, 0x34, 0x12, 0x00, 0x00}, r, 0x1000);
    REQUIRE(d.bytes_consumed == 7);
    REQUIRE(d.stmts.size() == 3);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{0x223BULL, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::StoreMemTSO{1u, 0u, ir::OpSize::I64}});
}

TEST_CASE("decode 67 48 8B 05 disp32 as absolute addr32, not RIP-relative") {
    ir::Ref r = 0;
    auto d = decode_ok({0x67, 0x48, 0x8B, 0x05, 0x34, 0x12, 0x00, 0x00}, r, 0x1000);
    REQUIRE(d.bytes_consumed == 8);
    REQUIRE(d.stmts.size() == 5);
    REQUIRE(d.stmts[0].op == ir::Op{ir::Constant{0x1234ULL, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{0xFFFF'FFFFULL, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::And, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op == ir::Op{ir::LoadMemTSO{2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op == ir::Op{ir::StoreReg{ir::Gpr::Rax, 3u, ir::OpSize::I64}});
}

TEST_CASE("decode 67 48 8B 05 FC FF FF FF zero-extends negative disp32") {
    ir::Ref r = 0;
    auto d = decode_ok({0x67, 0x48, 0x8B, 0x05, 0xFC, 0xFF, 0xFF, 0xFF}, r);
    REQUIRE(d.bytes_consumed == 8);
    REQUIRE(d.stmts.size() == 5);
    REQUIRE(d.stmts[0].op ==
            ir::Op{ir::Constant{0xFFFF'FFFF'FFFF'FFFCULL, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{0xFFFF'FFFFULL, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::And, 0u, 1u, ir::OpSize::I64}});
}

TEST_CASE("Preserved — decoder still rejects the HLT opcode (F4)") {
    ir::Ref r = 0;
    auto res = decode_any({0xF4}, r);
    REQUIRE(std::holds_alternative<DecodeError>(res));
    REQUIRE(std::get<DecodeError>(res) == DecodeError::UnsupportedEncoding);
}

TEST_CASE("decode MOV rax, [rbx + rcx*2 + 0x20] via SIB byte") {
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x8B, 0x44, 0x4B, 0x20}, r);
    REQUIRE(d.bytes_consumed == 5);
    REQUIRE(d.stmts.size() == 9);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rcx, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::Constant{1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Shl, 1u, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Add, 0u, 3u, ir::OpSize::I64}});
    REQUIRE(d.stmts[5].op == ir::Op{ir::Constant{0x20u, ir::OpSize::I64}});
    REQUIRE(d.stmts[6].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Add, 4u, 5u, ir::OpSize::I64}});
    REQUIRE(d.stmts[7].op == ir::Op{ir::LoadMemTSO{6u, ir::OpSize::I64}});
    REQUIRE(d.stmts[8].op == ir::Op{ir::StoreReg{ir::Gpr::Rax, 7u, ir::OpSize::I64}});
}

TEST_CASE("decode 67 48 8B 03 masks EBX for addr32 base-only memory") {
    ir::Ref r = 0;
    auto d = decode_ok({0x67, 0x48, 0x8B, 0x03}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 5);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{0xFFFF'FFFFULL, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::And, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op == ir::Op{ir::LoadMemTSO{2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op == ir::Op{ir::StoreReg{ir::Gpr::Rax, 3u, ir::OpSize::I64}});
}

TEST_CASE("decode 67 49 8B 00 keeps REX.B base extension under addr32") {
    ir::Ref r = 0;
    auto d = decode_ok({0x67, 0x49, 0x8B, 0x00}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 5);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::R8, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{0xFFFF'FFFFULL, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::And, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op == ir::Op{ir::LoadMemTSO{2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op == ir::Op{ir::StoreReg{ir::Gpr::Rax, 3u, ir::OpSize::I64}});
}

TEST_CASE("decode 67 48 8B 44 4B 20 masks the final SIB address to 32 bits") {
    ir::Ref r = 0;
    auto d = decode_ok({0x67, 0x48, 0x8B, 0x44, 0x4B, 0x20}, r);
    REQUIRE(d.bytes_consumed == 6);
    REQUIRE(d.stmts.size() == 11);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rcx, ir::OpSize::I64}});
    REQUIRE(d.stmts[7].op == ir::Op{ir::Constant{0xFFFF'FFFFULL, ir::OpSize::I64}});
    REQUIRE(d.stmts[8].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::And, 6u, 7u, ir::OpSize::I64}});
    REQUIRE(d.stmts[9].op == ir::Op{ir::LoadMemTSO{8u, ir::OpSize::I64}});
    REQUIRE(d.stmts[10].op == ir::Op{ir::StoreReg{ir::Gpr::Rax, 9u, ir::OpSize::I64}});
}

TEST_CASE("decode MOV rcx, [rax + r12] via REX.X SIB index extension") {
    ir::Ref r = 0;
    auto d = decode_ok({0x4A, 0x8B, 0x0C, 0x20}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 5);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::R12, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op == ir::Op{ir::LoadMemTSO{2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op == ir::Op{ir::StoreReg{ir::Gpr::Rcx, 3u, ir::OpSize::I64}});
}

TEST_CASE("decode 67 4A 8B 0C 20 keeps REX.X index extension under addr32") {
    ir::Ref r = 0;
    auto d = decode_ok({0x67, 0x4A, 0x8B, 0x0C, 0x20}, r);
    REQUIRE(d.bytes_consumed == 5);
    REQUIRE(d.stmts.size() == 7);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::R12, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op == ir::Op{ir::Constant{0xFFFF'FFFFULL, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::And, 2u, 3u, ir::OpSize::I64}});
    REQUIRE(d.stmts[5].op == ir::Op{ir::LoadMemTSO{4u, ir::OpSize::I64}});
    REQUIRE(d.stmts[6].op == ir::Op{ir::StoreReg{ir::Gpr::Rcx, 5u, ir::OpSize::I64}});
}

TEST_CASE("decode 67 AA uses EDI semantics for STOSB") {
    ir::Ref r = 0;
    auto d = decode_ok({0x67, 0xAA}, r);
    REQUIRE(d.bytes_consumed == 2);
    REQUIRE(d.stmts.size() == 10);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I8}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rdi, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::Constant{0xFFFF'FFFFULL, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::And, 1u, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op == ir::Op{ir::StoreMemTSO{3u, 0u, ir::OpSize::I8}});
    REQUIRE(d.stmts[8].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::And, 5u, 6u, ir::OpSize::I64}});
    REQUIRE(d.stmts[9].op == ir::Op{ir::StoreReg{ir::Gpr::Rdi, 7u, ir::OpSize::I64}});
}

TEST_CASE("decode CS override as a no-op on MOV rax, [rbx]") {
    ir::Ref r = 0;
    auto d = decode_ok({0x2E, 0x48, 0x8B, 0x03}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 3);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadMemTSO{0u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::StoreReg{ir::Gpr::Rax, 1u, ir::OpSize::I64}});
}

TEST_CASE("Error: FS/GS segment base accesses remain unsupported") {
    ir::Ref r = 0;
    auto res = decode_any({0x65, 0x48, 0x8B, 0x03}, r);
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

TEST_CASE("decode FF /4 r/m64 register form → JumpReg") {
    // 48 FF E0 → jmp rax (indirect through rax).
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0xFF, 0xE0}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 2);
    REQUIRE(d.stmts[0].result == std::optional<ir::Ref>(0u));
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].result == std::nullopt);
    REQUIRE(d.stmts[1].op == ir::Op{ir::JumpReg{0u}});
}

TEST_CASE("decode FF /4 r/m64 memory form → JumpReg") {
    // 48 FF 63 10 → jmp qword ptr [rbx + 0x10].
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0xFF, 0x63, 0x10}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 5);
    REQUIRE(d.stmts[0].result == std::optional<ir::Ref>(1u));
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].result == std::optional<ir::Ref>(2u));
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{0x10u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].result == std::optional<ir::Ref>(3u));
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Add, 1u, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].result == std::optional<ir::Ref>(0u));
    REQUIRE(d.stmts[3].op == ir::Op{ir::LoadMemTSO{3u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].result == std::nullopt);
    REQUIRE(d.stmts[4].op == ir::Op{ir::JumpReg{0u}});
}

TEST_CASE("decode CALL rel32 computes absolute target from instruction_guest_pc") {
    // E8 00 01 00 00 → call +0x100 from end-of-instruction
    // with start PC = 0x1000 -> target = 0x1105.
    ir::Ref r = 0;
    const std::vector<std::uint8_t> bytes = {0xE8, 0x00, 0x01, 0x00, 0x00};
    auto res = decoder::decode_one(std::span<const std::uint8_t>{bytes}, r, 0x1000);
    REQUIRE(std::holds_alternative<Decoded>(res));
    const auto& d = std::get<Decoded>(res);
    REQUIRE(d.bytes_consumed == 5);
    REQUIRE(d.stmts.size() == 1);
    REQUIRE(d.stmts[0].op == ir::Op{ir::JumpRel{0x1105}});
    REQUIRE(r == 0);
}

TEST_CASE("decode FF /2 r/m64 register form → JumpReg (CALL placeholder)") {
    // 48 FF D0 → call rax (indirect through rax).
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0xFF, 0xD0}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 2);
    REQUIRE(d.stmts[0].result == std::optional<ir::Ref>(0u));
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].result == std::nullopt);
    REQUIRE(d.stmts[1].op == ir::Op{ir::JumpReg{0u}});
}

TEST_CASE("decode FF /2 r/m64 memory form → JumpReg (CALL placeholder)") {
    // 48 FF 53 10 → call qword ptr [rbx + 0x10].
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0xFF, 0x53, 0x10}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 5);
    REQUIRE(d.stmts[0].result == std::optional<ir::Ref>(1u));
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].result == std::optional<ir::Ref>(2u));
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{0x10u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].result == std::optional<ir::Ref>(3u));
    REQUIRE(d.stmts[2].op == ir::Op{ir::BinOp{ir::BinOpKind::Add, 1u, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].result == std::optional<ir::Ref>(0u));
    REQUIRE(d.stmts[3].op == ir::Op{ir::LoadMemTSO{3u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].result == std::nullopt);
    REQUIRE(d.stmts[4].op == ir::Op{ir::JumpReg{0u}});
}

TEST_CASE("decode LEAVE (0xC9) → RSP := RBP; RBP := [RBP]") {
    ir::Ref r = 0;
    auto d = decode_ok({0xC9}, r);
    REQUIRE(d.bytes_consumed == 1);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rbp, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::StoreReg{ir::Gpr::Rsp, 0u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::LoadMemTSO{0u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op == ir::Op{ir::StoreReg{ir::Gpr::Rbp, 1u, ir::OpSize::I64}});
    REQUIRE(r == 2);
}

TEST_CASE("decode RET imm16 (C2 iw) adjusts RSP then returns") {
    ir::Ref r = 0;
    auto d = decode_ok({0xC2, 0x04, 0x00}, r);  // pop 4 bytes -> add 12 total
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 5);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rsp, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{12u, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op == ir::Op{ir::StoreReg{ir::Gpr::Rsp, 2u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op == ir::Op{ir::Return{}});
    REQUIRE(r == 3);
}

TEST_CASE("decode all supported Jcc rel8 opcodes (70-7F) into CondJumpRel") {
    // Decode map: 70-77 are supported, 78-79 are unsupported in this slice.
    constexpr std::array<std::optional<ir::CondCode>, 16> kCondMap = {
        ir::CondCode::Ov, ir::CondCode::NoOv, ir::CondCode::Nc,
        ir::CondCode::Cc, ir::CondCode::Eq, ir::CondCode::Ne,
        ir::CondCode::Ule, ir::CondCode::Ugt, ir::CondCode::Mi,
        ir::CondCode::Pl, std::nullopt, std::nullopt,
        ir::CondCode::Slt, ir::CondCode::Sge, ir::CondCode::Sle,
        ir::CondCode::Sgt};

    for (std::size_t i = 0; i < kCondMap.size(); ++i) {
        const std::array<std::uint8_t, 2> bytes = {
            static_cast<std::uint8_t>(0x70u + i), 0x10u};
        const std::uint64_t pc = 0x7000u + 0x20u * i;
        const std::uint64_t target = pc + 2u + 0x10u;
        const std::uint64_t fallthrough = pc + 2u;

        ir::Ref next = 0;
        auto res = decode_one(std::span<const std::uint8_t>{bytes}, next, pc);
        if (kCondMap[i].has_value()) {
            REQUIRE(std::holds_alternative<Decoded>(res));
            const auto& d = std::get<Decoded>(res);
            REQUIRE(d.bytes_consumed == 2u);
            REQUIRE(d.stmts.size() == 1u);
            REQUIRE(d.stmts[0].op ==
                    ir::Op{ir::CondJumpRel{*kCondMap[i], target, fallthrough}});
        } else {
            REQUIRE(std::holds_alternative<DecodeError>(res));
            REQUIRE(std::get<DecodeError>(res) == DecodeError::UnsupportedEncoding);
        }
    }
}

TEST_CASE("decode all supported Jcc rel32 opcodes (0F 80-8F) into CondJumpRel") {
    constexpr std::array<std::optional<ir::CondCode>, 16> kCondMap = {
        ir::CondCode::Ov, ir::CondCode::NoOv, ir::CondCode::Nc,
        ir::CondCode::Cc, ir::CondCode::Eq, ir::CondCode::Ne,
        ir::CondCode::Ule, ir::CondCode::Ugt, ir::CondCode::Mi,
        ir::CondCode::Pl, std::nullopt, std::nullopt,
        ir::CondCode::Slt, ir::CondCode::Sge, ir::CondCode::Sle,
        ir::CondCode::Sgt};

    for (std::size_t i = 0; i < kCondMap.size(); ++i) {
        const std::array<std::uint8_t, 6> bytes = {
            0x0Fu, static_cast<std::uint8_t>(0x80u + i),
            0x20u, 0x00u, 0x00u, 0x00u};
        const std::uint64_t pc = 0x9000u + 0x30u * i;
        const std::uint64_t target = pc + 6u + 0x20u;
        const std::uint64_t fallthrough = pc + 6u;

        ir::Ref next = 0;
        auto res = decode_one(std::span<const std::uint8_t>{bytes}, next, pc);
        if (kCondMap[i].has_value()) {
            REQUIRE(std::holds_alternative<Decoded>(res));
            const auto& d = std::get<Decoded>(res);
            REQUIRE(d.bytes_consumed == 6u);
            REQUIRE(d.stmts.size() == 1u);
            REQUIRE(d.stmts[0].op ==
                    ir::Op{ir::CondJumpRel{*kCondMap[i], target, fallthrough}});
        } else {
            REQUIRE(std::holds_alternative<DecodeError>(res));
            REQUIRE(std::get<DecodeError>(res) == DecodeError::UnsupportedEncoding);
        }
    }
}

TEST_CASE("decode PF/NPF Jcc opcodes as unsupportedEncoding") {
    const std::array<std::uint8_t, 2> rel8_pf = {0x7Au, 0x00u};
    const std::array<std::uint8_t, 2> rel8_npf = {0x7Bu, 0x00u};
    const std::array<std::uint8_t, 6> rel32_pf = {0x0Fu, 0x8Au, 0x00u, 0x00u, 0x00u, 0x00u};
    const std::array<std::uint8_t, 6> rel32_npf = {0x0Fu, 0x8Bu, 0x00u, 0x00u, 0x00u, 0x00u};

    ir::Ref r = 0;
    auto r_pf8 = decode_one(std::span<const std::uint8_t>{rel8_pf}, r, 0x8000);
    REQUIRE(std::holds_alternative<DecodeError>(r_pf8));
    REQUIRE(std::get<DecodeError>(r_pf8) == DecodeError::UnsupportedEncoding);

    auto r_npf8 = decode_one(std::span<const std::uint8_t>{rel8_npf}, r, 0x8000);
    REQUIRE(std::holds_alternative<DecodeError>(r_npf8));
    REQUIRE(std::get<DecodeError>(r_npf8) == DecodeError::UnsupportedEncoding);

    auto r_pf32 = decode_one(std::span<const std::uint8_t>{rel32_pf}, r, 0x8000);
    REQUIRE(std::holds_alternative<DecodeError>(r_pf32));
    REQUIRE(std::get<DecodeError>(r_pf32) == DecodeError::UnsupportedEncoding);

    auto r_npf32 = decode_one(std::span<const std::uint8_t>{rel32_npf}, r, 0x8000);
    REQUIRE(std::holds_alternative<DecodeError>(r_npf32));
    REQUIRE(std::get<DecodeError>(r_npf32) == DecodeError::UnsupportedEncoding);
}

TEST_CASE("decode all supported CMOVcc (0F 40-4F) into Select") {
    constexpr std::array<std::optional<ir::CondCode>, 16> kCondMap = {
        ir::CondCode::Ov, ir::CondCode::NoOv, ir::CondCode::Nc,
        ir::CondCode::Cc, ir::CondCode::Eq, ir::CondCode::Ne,
        ir::CondCode::Ule, ir::CondCode::Ugt, ir::CondCode::Mi,
        ir::CondCode::Pl, std::nullopt, std::nullopt,
        ir::CondCode::Slt, ir::CondCode::Sge, ir::CondCode::Sle,
        ir::CondCode::Sgt};

    for (std::size_t i = 0; i < kCondMap.size(); ++i) {
        const std::array<std::uint8_t, 4> bytes = {
            0x48u, 0x0Fu, static_cast<std::uint8_t>(0x40u + i), 0xC3u};
        ir::Ref next = 0;
        auto res = decode_one(std::span<const std::uint8_t>{bytes}, next, 0x1000);
        if (kCondMap[i].has_value()) {
            REQUIRE(std::holds_alternative<Decoded>(res));
            const auto& d = std::get<Decoded>(res);
            REQUIRE(d.bytes_consumed == 4u);
            REQUIRE(d.stmts.size() == 4u);
            REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
            REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
            REQUIRE(d.stmts[2].op ==
                    ir::Op{ir::Select{*kCondMap[i], 1u, 0u, ir::OpSize::I64}});
            REQUIRE(d.stmts[3].op ==
                    ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});
        } else {
            REQUIRE(std::holds_alternative<DecodeError>(res));
            REQUIRE(std::get<DecodeError>(res) == DecodeError::UnsupportedEncoding);
        }
    }
}

TEST_CASE("decode CMOVNE rax, rbx via 48 0F 45 C3 as Select") {
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x0F, 0x45, 0xC3}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 4);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op ==
            ir::Op{ir::Select{ir::CondCode::Ne, 1u, 0u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});
}

TEST_CASE("decode all supported SETcc (0F 90-9F) into Select") {
    constexpr std::array<std::optional<ir::CondCode>, 16> kCondMap = {
        ir::CondCode::Ov, ir::CondCode::NoOv, ir::CondCode::Nc,
        ir::CondCode::Cc, ir::CondCode::Eq, ir::CondCode::Ne,
        ir::CondCode::Ule, ir::CondCode::Ugt, ir::CondCode::Mi,
        ir::CondCode::Pl, std::nullopt, std::nullopt,
        ir::CondCode::Slt, ir::CondCode::Sge, ir::CondCode::Sle,
        ir::CondCode::Sgt};

    for (std::size_t i = 0; i < kCondMap.size(); ++i) {
        const std::array<std::uint8_t, 3> bytes = {0x0Fu, static_cast<std::uint8_t>(0x90u + i), 0xC0u};
        ir::Ref next = 0;
        auto res = decode_one(std::span<const std::uint8_t>{bytes}, next, 0x2000);
        if (kCondMap[i].has_value()) {
            REQUIRE(std::holds_alternative<Decoded>(res));
            const auto& d = std::get<Decoded>(res);
            REQUIRE(d.bytes_consumed == 3u);
            REQUIRE(d.stmts.size() == 4u);
            REQUIRE(d.stmts[0].op == ir::Op{ir::Constant{1u, ir::OpSize::I8}});
            REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{0u, ir::OpSize::I8}});
            REQUIRE(d.stmts[2].op ==
                    ir::Op{ir::Select{*kCondMap[i], 0u, 1u, ir::OpSize::I8}});
            REQUIRE(d.stmts[3].op == ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I8}});
        } else {
            REQUIRE(std::holds_alternative<DecodeError>(res));
            REQUIRE(std::get<DecodeError>(res) == DecodeError::UnsupportedEncoding);
        }
    }
}

TEST_CASE("decode CMOVGT rax, byte from [rbx] via 48 0F 4F 03") {
    ir::Ref r = 0;
    auto d = decode_ok({0x48, 0x0F, 0x4F, 0x03}, r);
    REQUIRE(d.bytes_consumed == 4);
    REQUIRE(d.stmts.size() == 5);
    REQUIRE(d.stmts[0].op == ir::Op{ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::LoadMemTSO{1u, ir::OpSize::I64}});
    REQUIRE(d.stmts[3].op ==
            ir::Op{ir::Select{ir::CondCode::Sgt, 2u, 0u, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 3u, ir::OpSize::I64}});
}

TEST_CASE("decode SETNE [rbx] via memory form 0F 95 03") {
    ir::Ref r = 0;
    auto d = decode_ok({0x0F, 0x95, 0x03}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(d.stmts.size() == 5);
    REQUIRE(d.stmts[0].op == ir::Op{ir::Constant{1u, ir::OpSize::I8}});
    REQUIRE(d.stmts[1].op == ir::Op{ir::Constant{0u, ir::OpSize::I8}});
    REQUIRE(d.stmts[2].op == ir::Op{ir::Select{ir::CondCode::Ne, 0u, 1u, ir::OpSize::I8}});
    REQUIRE(d.stmts[3].op == ir::Op{ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}});
    REQUIRE(d.stmts[4].op == ir::Op{ir::StoreMemTSO{3u, 2u, ir::OpSize::I8}});
}

TEST_CASE("decode PF/NPF SETcc opcodes as unsupportedEncoding") {
    const std::array<std::uint8_t, 3> set_pf = {0x0Fu, 0x9Au, 0xC0u};
    const std::array<std::uint8_t, 3> set_npf = {0x0Fu, 0x9Bu, 0xC0u};

    ir::Ref r = 0;
    auto r_pf = decode_one(std::span<const std::uint8_t>{set_pf}, r, 0x9000);
    REQUIRE(std::holds_alternative<DecodeError>(r_pf));
    REQUIRE(std::get<DecodeError>(r_pf) == DecodeError::UnsupportedEncoding);

    auto r_npf = decode_one(std::span<const std::uint8_t>{set_npf}, r, 0x9000);
    REQUIRE(std::holds_alternative<DecodeError>(r_npf));
    REQUIRE(std::get<DecodeError>(r_npf) == DecodeError::UnsupportedEncoding);
}

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

// ---------------------------------------------------------------------
// F1-DC-066 REP / REPE / REPNE prefixes on string ops
// ---------------------------------------------------------------------

TEST_CASE("decode REP STOSB (F3 AA) → RepStos IR (F2-BK-008)") {
    ir::Ref r = 0;
    auto d = decode_ok({0xF3, 0xAA}, r);
    REQUIRE(d.bytes_consumed == 2);
    REQUIRE(d.stmts.size() == 1);
    REQUIRE(std::holds_alternative<ir::RepStos>(d.stmts[0].op));
    const auto& rs = std::get<ir::RepStos>(d.stmts[0].op);
    REQUIRE(rs.size == ir::OpSize::I8);
    REQUIRE(rs.reverse == false);
}

TEST_CASE("decode REPNE SCASB (F2 AE) → InlineAsm placeholder, 2 bytes") {
    ir::Ref r = 0;
    auto d = decode_ok({0xF2, 0xAE}, r);
    REQUIRE(d.bytes_consumed == 2);
    REQUIRE(std::holds_alternative<ir::InlineAsm>(d.stmts[0].op));
}

TEST_CASE("decode REP MOVSQ (F3 48 A5) → RepMovs IR (F2-BK-009)") {
    ir::Ref r = 0;
    auto d = decode_ok({0xF3, 0x48, 0xA5}, r);
    REQUIRE(d.bytes_consumed == 3);
    REQUIRE(std::holds_alternative<ir::RepMovs>(d.stmts[0].op));
    const auto& rm = std::get<ir::RepMovs>(d.stmts[0].op);
    REQUIRE(rm.size == ir::OpSize::I64);
    REQUIRE(rm.reverse == false);
}

TEST_CASE("decode REPE CMPSB (F3 A6) → InlineAsm placeholder") {
    ir::Ref r = 0;
    auto d = decode_ok({0xF3, 0xA6}, r);
    REQUIRE(std::holds_alternative<ir::InlineAsm>(d.stmts[0].op));
}

TEST_CASE("decode plain STOSB (no REP) still produces real IR (regression)") {
    ir::Ref r = 0;
    auto d = decode_ok({0xAA}, r);
    REQUIRE_FALSE(std::holds_alternative<ir::InlineAsm>(d.stmts[0].op));
}
