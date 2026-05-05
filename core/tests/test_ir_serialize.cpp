// core/tests/test_ir_serialize.cpp — round-trip tests for the IR
// binary serializer (F1-IR-017 / F1-IR-018, RFC 0009).
//
// Coverage:
//   * Per-Op-variant round-trip (one TEST_CASE per variant).
//   * Mixed program with all current variants in one stream.
//   * CRC corruption detection (single-byte flip).
//   * Truncation detection (chop the last few bytes).
//   * Function (multiple BasicBlocks) round-trip.

#include <catch2/catch_test_macros.hpp>

#include <cstdint>
#include <span>
#include <vector>

#include "prisma/ir.hpp"
#include "prisma/ir_serialize.hpp"

using namespace prisma::ir;

namespace {

// Round-trip a single statement and assert structural equality.
void check_single_stmt_roundtrip(const Stmt& s) {
    const std::vector<Stmt> in = {s};
    const auto bytes = serialize(in);
    REQUIRE(bytes.size() >= 16);
    auto r = deserialize_stmts(std::span<const std::uint8_t>(bytes));
    REQUIRE(r.error == DeserializeError::Ok);
    REQUIRE(r.stmts.size() == 1u);
    REQUIRE(r.stmts[0] == s);
}

}  // namespace

TEST_CASE("ir_serialize: Constant round-trip", "[ir_serialize]") {
    check_single_stmt_roundtrip(Stmt{Ref{0u},
        Op{Constant{0xDEAD'BEEF'CAFE'F00DULL, OpSize::I64}}});
    check_single_stmt_roundtrip(Stmt{Ref{1u},
        Op{Constant{0x42u, OpSize::I8}}});
}

TEST_CASE("ir_serialize: LoadReg round-trip", "[ir_serialize]") {
    check_single_stmt_roundtrip(Stmt{Ref{2u},
        Op{LoadReg{Gpr::R13, OpSize::I32}}});
}

TEST_CASE("ir_serialize: StoreReg round-trip", "[ir_serialize]") {
    check_single_stmt_roundtrip(Stmt{std::nullopt,
        Op{StoreReg{Gpr::R8, Ref{7u}, OpSize::I64}}});
}

TEST_CASE("ir_serialize: LoadSegBase round-trip", "[ir_serialize]") {
    check_single_stmt_roundtrip(Stmt{Ref{3u},
        Op{LoadSegBase{SegmentReg::Gs}}});
    check_single_stmt_roundtrip(Stmt{Ref{4u},
        Op{LoadSegBase{SegmentReg::Fs}}});
}

TEST_CASE("ir_serialize: BinOp round-trip", "[ir_serialize]") {
    check_single_stmt_roundtrip(Stmt{Ref{5u},
        Op{BinOp{BinOpKind::Mul, Ref{1u}, Ref{2u}, OpSize::I64}}});
    check_single_stmt_roundtrip(Stmt{Ref{6u},
        Op{BinOp{BinOpKind::Rcr, Ref{3u}, Ref{4u}, OpSize::I16}}});
}

TEST_CASE("ir_serialize: Compare round-trip", "[ir_serialize]") {
    check_single_stmt_roundtrip(Stmt{Ref{7u},
        Op{Compare{CondCode::Slt, Ref{1u}, Ref{2u}, OpSize::I32}}});
}

TEST_CASE("ir_serialize: Select round-trip", "[ir_serialize]") {
    check_single_stmt_roundtrip(Stmt{Ref{8u},
        Op{Select{CondCode::Eq, Ref{3u}, Ref{4u}, OpSize::I64}}});
}

TEST_CASE("ir_serialize: LoadMem / StoreMem round-trip", "[ir_serialize]") {
    check_single_stmt_roundtrip(Stmt{Ref{9u},
        Op{LoadMem{Ref{10u}, OpSize::I64}}});
    check_single_stmt_roundtrip(Stmt{std::nullopt,
        Op{StoreMem{Ref{11u}, Ref{12u}, OpSize::I8}}});
}

TEST_CASE("ir_serialize: LoadMemTSO / StoreMemTSO round-trip", "[ir_serialize]") {
    check_single_stmt_roundtrip(Stmt{Ref{13u},
        Op{LoadMemTSO{Ref{14u}, OpSize::I32}}});
    check_single_stmt_roundtrip(Stmt{std::nullopt,
        Op{StoreMemTSO{Ref{15u}, Ref{16u}, OpSize::I64}}});
}

TEST_CASE("ir_serialize: Jump / CondJump / Return round-trip", "[ir_serialize]") {
    check_single_stmt_roundtrip(Stmt{std::nullopt,
        Op{Jump{42u}}});
    check_single_stmt_roundtrip(Stmt{std::nullopt,
        Op{CondJump{Ref{17u}, 1u, 2u}}});
    check_single_stmt_roundtrip(Stmt{std::nullopt,
        Op{Return{}}});
}

TEST_CASE("ir_serialize: JumpReg / CmpFlags round-trip", "[ir_serialize]") {
    check_single_stmt_roundtrip(Stmt{std::nullopt,
        Op{JumpReg{Ref{99u}}}});
    check_single_stmt_roundtrip(Stmt{std::nullopt,
        Op{CmpFlags{Ref{18u}, Ref{19u}, OpSize::I64}}});
}

TEST_CASE("ir_serialize: JumpRel / CondJumpRel round-trip", "[ir_serialize]") {
    check_single_stmt_roundtrip(Stmt{std::nullopt,
        Op{JumpRel{0x1000'2000'3000'4000ULL}}});
    check_single_stmt_roundtrip(Stmt{std::nullopt,
        Op{CondJumpRel{CondCode::Ne, 0x4000ULL, 0x4010ULL}}});
}

TEST_CASE("ir_serialize: CallRel / CallReg / RetAdjusted round-trip", "[ir_serialize]") {
    check_single_stmt_roundtrip(Stmt{std::nullopt,
        Op{CallRel{0x1000ULL, 0x1005ULL}}});
    check_single_stmt_roundtrip(Stmt{std::nullopt,
        Op{CallReg{Ref{20u}, 0x2010ULL}}});
    check_single_stmt_roundtrip(Stmt{std::nullopt,
        Op{RetAdjusted{8ULL}}});
}

TEST_CASE("ir_serialize: Cpuid / Syscall / Trap round-trip", "[ir_serialize]") {
    check_single_stmt_roundtrip(Stmt{std::nullopt, Op{Cpuid{}}});
    check_single_stmt_roundtrip(Stmt{std::nullopt, Op{Syscall{}}});
    check_single_stmt_roundtrip(Stmt{std::nullopt,
        Op{Trap{TrapKind::Sigill}}});
}

TEST_CASE("ir_serialize: Extend / Truncate / Fence round-trip", "[ir_serialize]") {
    check_single_stmt_roundtrip(Stmt{Ref{1u},
        Op{Extend{Ref{0u}, OpSize::I8, OpSize::I64, /*signed=*/true}}});
    check_single_stmt_roundtrip(Stmt{Ref{2u},
        Op{Extend{Ref{0u}, OpSize::I32, OpSize::I64, /*signed=*/false}}});
    check_single_stmt_roundtrip(Stmt{Ref{3u},
        Op{Truncate{Ref{0u}, OpSize::I8}}});
    check_single_stmt_roundtrip(Stmt{std::nullopt,
        Op{Fence{FenceKind::Mfence}}});
    check_single_stmt_roundtrip(Stmt{std::nullopt,
        Op{Fence{FenceKind::Lfence}}});
    check_single_stmt_roundtrip(Stmt{std::nullopt,
        Op{Fence{FenceKind::Sfence}}});
}

TEST_CASE("ir_serialize: GuestPc round-trip", "[ir_serialize]") {
    check_single_stmt_roundtrip(Stmt{std::nullopt,
        Op{GuestPc{0x0000'0000'0040'1000ULL}}});
    check_single_stmt_roundtrip(Stmt{std::nullopt,
        Op{GuestPc{0xFFFF'FFFF'FFFF'FFFFULL}}});  // edge: max u64
    check_single_stmt_roundtrip(Stmt{std::nullopt,
        Op{GuestPc{0u}}});                         // edge: zero
}

TEST_CASE("ir_serialize: VecConstant + VecBinOp round-trip",
          "[ir_serialize]") {
    check_single_stmt_roundtrip(Stmt{Ref{1u},
        Op{VecConstant{0x0102'0304'0506'0708ULL, 0x090A'0B0C'0D0E'0F10ULL}}});
    check_single_stmt_roundtrip(Stmt{Ref{2u},
        Op{VecBinOp{VecBinOpKind::Add, Ref{0u}, Ref{1u}, VecLane::B16}}});
    check_single_stmt_roundtrip(Stmt{Ref{3u},
        Op{VecBinOp{VecBinOpKind::Xor, Ref{0u}, Ref{0u}, VecLane::D2}}});
}

TEST_CASE("ir_serialize: RspAdjust round-trip preserves signed delta",
          "[ir_serialize]") {
    check_single_stmt_roundtrip(Stmt{std::nullopt, Op{RspAdjust{-8}}});
    check_single_stmt_roundtrip(Stmt{std::nullopt, Op{RspAdjust{0}}});
    check_single_stmt_roundtrip(Stmt{std::nullopt, Op{RspAdjust{16}}});
    check_single_stmt_roundtrip(Stmt{std::nullopt, Op{RspAdjust{0x7FFF'FFFF'FFFFLL}}});
}

TEST_CASE("ir_serialize: WriteFlags + ReadFlag + CondJumpFlags round-trip",
          "[ir_serialize]") {
    check_single_stmt_roundtrip(Stmt{Ref{1u},
        Op{WriteFlags{BinOpKind::Sub, Ref{0u}, Ref{0u}, OpSize::I64}}});
    check_single_stmt_roundtrip(Stmt{Ref{2u},
        Op{ReadFlag{Ref{1u}, FlagBit::Zero}}});
    check_single_stmt_roundtrip(Stmt{Ref{3u},
        Op{ReadFlag{Ref{1u}, FlagBit::Carry}}});
    check_single_stmt_roundtrip(Stmt{Ref{4u},
        Op{ReadFlag{Ref{1u}, FlagBit::Overflow}}});
    check_single_stmt_roundtrip(Stmt{std::nullopt,
        Op{CondJumpFlags{Ref{1u}, CondCode::Eq, 5u, 6u}}});
}

TEST_CASE("ir_serialize: FpConstant + FpBinOp round-trip", "[ir_serialize]") {
    check_single_stmt_roundtrip(Stmt{Ref{1u},
        Op{FpConstant{0x3FF0'0000'0000'0000ULL, FpSize::F64}}});  // 1.0 double
    check_single_stmt_roundtrip(Stmt{Ref{2u},
        Op{FpConstant{0x3F80'0000ULL, FpSize::F32}}});            // 1.0 float
    check_single_stmt_roundtrip(Stmt{Ref{3u},
        Op{FpBinOp{FpBinOpKind::Add, Ref{1u}, Ref{2u}, FpSize::F64}}});
    check_single_stmt_roundtrip(Stmt{Ref{4u},
        Op{FpBinOp{FpBinOpKind::Div, Ref{1u}, Ref{2u}, FpSize::F32}}});
}

TEST_CASE("ir_serialize: InlineAsm round-trip preserves the byte payload",
          "[ir_serialize]") {
    check_single_stmt_roundtrip(Stmt{std::nullopt,
        Op{InlineAsm{{0x0F, 0x05}}}});               // SYSCALL
    check_single_stmt_roundtrip(Stmt{std::nullopt,
        Op{InlineAsm{{}}}});                         // edge: empty
    std::vector<std::uint8_t> long_bytes(1024u, 0xCCu);
    check_single_stmt_roundtrip(Stmt{std::nullopt,
        Op{InlineAsm{long_bytes}}});                 // edge: large
}

TEST_CASE("ir_serialize: large mixed program round-trip", "[ir_serialize]") {
    const std::vector<Stmt> program = {
        // 0-3: pure values
        {Ref{0u}, Op{Constant{0xCAFEULL, OpSize::I64}}},
        {Ref{1u}, Op{LoadReg{Gpr::Rax, OpSize::I64}}},
        {Ref{2u}, Op{LoadSegBase{SegmentReg::Fs}}},
        {Ref{3u}, Op{BinOp{BinOpKind::Add, Ref{0u}, Ref{1u}, OpSize::I64}}},

        // 4-6: comparison + branch
        {Ref{4u}, Op{Compare{CondCode::Eq, Ref{1u}, Ref{0u}, OpSize::I64}}},
        {Ref{5u}, Op{Select{CondCode::Ne, Ref{0u}, Ref{1u}, OpSize::I64}}},
        {std::nullopt, Op{StoreReg{Gpr::Rbx, Ref{5u}, OpSize::I64}}},

        // 7-10: memory
        {Ref{7u}, Op{LoadMem{Ref{1u}, OpSize::I32}}},
        {std::nullopt, Op{StoreMem{Ref{1u}, Ref{7u}, OpSize::I32}}},
        {Ref{9u}, Op{LoadMemTSO{Ref{1u}, OpSize::I64}}},
        {std::nullopt, Op{StoreMemTSO{Ref{1u}, Ref{9u}, OpSize::I64}}},

        // 11-14: control flow MVP
        {std::nullopt, Op{CmpFlags{Ref{1u}, Ref{0u}, OpSize::I64}}},
        {std::nullopt, Op{Jump{0u}}},
        {std::nullopt, Op{CondJump{Ref{4u}, 1u, 2u}}},
        {std::nullopt, Op{Return{}}},

        // 15-18: indirect / guest-PC control flow
        {std::nullopt, Op{JumpReg{Ref{1u}}}},
        {std::nullopt, Op{JumpRel{0x4000ULL}}},
        {std::nullopt, Op{CondJumpRel{CondCode::Slt, 0x4000ULL, 0x4010ULL}}},
        {std::nullopt, Op{CallRel{0x1000ULL, 0x1005ULL}}},

        // 19-22: indirect call + returns + placeholders
        {std::nullopt, Op{CallReg{Ref{1u}, 0x2010ULL}}},
        {std::nullopt, Op{RetAdjusted{0u}}},
        {std::nullopt, Op{Cpuid{}}},
        {std::nullopt, Op{Syscall{}}},

        // 23: trap
        {std::nullopt, Op{Trap{TrapKind::Sigfpe}}},
    };

    const auto bytes = serialize(program);
    auto r = deserialize_stmts(std::span<const std::uint8_t>(bytes));
    REQUIRE(r.error == DeserializeError::Ok);
    REQUIRE(r.stmts.size() == program.size());
    for (std::size_t i = 0; i < program.size(); ++i) {
        REQUIRE(r.stmts[i] == program[i]);
    }
}

TEST_CASE("ir_serialize: single-byte flip yields BadCrc", "[ir_serialize]") {
    const std::vector<Stmt> program = {
        {Ref{0u}, Op{Constant{0x1111'2222'3333'4444ULL, OpSize::I64}}},
        {Ref{1u}, Op{LoadReg{Gpr::Rcx, OpSize::I64}}},
        {Ref{2u}, Op{BinOp{BinOpKind::Xor, Ref{0u}, Ref{1u}, OpSize::I64}}},
    };
    auto bytes = serialize(program);
    REQUIRE(bytes.size() > 16);

    // Flip a byte squarely in the middle of the payload, away from
    // the header (so we still see kSerializeMagic on the read path).
    const std::size_t mid = bytes.size() / 2;
    bytes[mid] ^= static_cast<std::uint8_t>(0x80u);

    auto r = deserialize_stmts(std::span<const std::uint8_t>(bytes));
    REQUIRE(r.error == DeserializeError::BadCrc);
    REQUIRE(r.stmts.empty());
}

TEST_CASE("ir_serialize: truncated stream yields Truncated", "[ir_serialize]") {
    const std::vector<Stmt> program = {
        {Ref{0u}, Op{Constant{0x1234ULL, OpSize::I64}}},
        {std::nullopt, Op{Return{}}},
    };
    auto bytes = serialize(program);

    // Below the 16-byte minimum envelope: pre-CRC check we report
    // Truncated, since at this size the stream cannot even hold a
    // header + count + crc trailer.
    bytes.resize(15);
    auto r = deserialize_stmts(std::span<const std::uint8_t>(bytes));
    REQUIRE(r.error == DeserializeError::Truncated);

    // Edge: empty stream is also Truncated.
    auto empty = deserialize_stmts(std::span<const std::uint8_t>{});
    REQUIRE(empty.error == DeserializeError::Truncated);
}

TEST_CASE("ir_serialize: bad magic", "[ir_serialize]") {
    auto bytes = serialize(std::vector<Stmt>{
        {Ref{0u}, Op{Constant{0u, OpSize::I64}}},
    });
    bytes[0] ^= static_cast<std::uint8_t>(0xFFu);
    auto r = deserialize_stmts(std::span<const std::uint8_t>(bytes));
    REQUIRE(r.error == DeserializeError::BadMagic);
}

TEST_CASE("ir_serialize: Function with two BasicBlocks round-trip",
          "[ir_serialize]") {
    Function fn;
    fn.entry = 0u;
    BasicBlock b0;
    b0.id = 0u;
    b0.stmts = {
        {Ref{0u}, Op{Constant{0x10ULL, OpSize::I64}}},
        {Ref{1u}, Op{LoadReg{Gpr::Rsi, OpSize::I64}}},
        {Ref{2u}, Op{BinOp{BinOpKind::Add, Ref{0u}, Ref{1u}, OpSize::I64}}},
        {std::nullopt, Op{Jump{1u}}},
    };
    BasicBlock b1;
    b1.id = 1u;
    b1.stmts = {
        {std::nullopt, Op{StoreReg{Gpr::Rdi, Ref{2u}, OpSize::I64}}},
        {std::nullopt, Op{Return{}}},
    };
    fn.blocks = {b0, b1};

    const auto bytes = serialize(fn);
    auto [err, got] = deserialize_function(std::span<const std::uint8_t>(bytes));
    REQUIRE(err == DeserializeError::Ok);
    REQUIRE(got.has_value());
    REQUIRE(got->entry == fn.entry);
    REQUIRE(got->blocks.size() == fn.blocks.size());
    for (std::size_t i = 0; i < fn.blocks.size(); ++i) {
        REQUIRE(got->blocks[i].id == fn.blocks[i].id);
        REQUIRE(got->blocks[i].stmts.size() == fn.blocks[i].stmts.size());
        for (std::size_t j = 0; j < fn.blocks[i].stmts.size(); ++j) {
            REQUIRE(got->blocks[i].stmts[j] == fn.blocks[i].stmts[j]);
        }
    }
}

TEST_CASE("ir_serialize: empty stmt list round-trip", "[ir_serialize]") {
    const std::vector<Stmt> empty;
    const auto bytes = serialize(empty);
    REQUIRE(bytes.size() == 4 + 2 + 2 + 4 + 4);  // magic+ver+rsv+count+crc
    auto r = deserialize_stmts(std::span<const std::uint8_t>(bytes));
    REQUIRE(r.error == DeserializeError::Ok);
    REQUIRE(r.stmts.empty());
}
