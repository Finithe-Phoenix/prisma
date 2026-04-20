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
