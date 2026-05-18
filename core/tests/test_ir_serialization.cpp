// core/tests/test_ir_serialization.cpp - F1-IR-017 serialization contract tests.
//
// These tests pin the public F1-IR-017 writer API. Deserialization is covered
// separately by F1-IR-018.

#include <catch2/catch_test_macros.hpp>

#include "prisma/ir.hpp"
#include "prisma/ir_serialization.hpp"

#include <algorithm>
#include <array>
#include <cstdint>
#include <optional>
#include <vector>

using namespace prisma::ir;

namespace {

Function simple_program() {
    Function fn;
    fn.entry = 0;
    fn.blocks.push_back(BasicBlock{
        0u,
        {
            {0u, Constant{10u, OpSize::I64}},
            {1u, Constant{32u, OpSize::I64}},
            {2u, BinOp{BinOpKind::Add, 0u, 1u, OpSize::I64}},
            {std::nullopt, Return{}},
        },
    });
    return fn;
}

std::uint16_t read_u16le(const std::vector<std::uint8_t>& bytes, std::size_t off) {
    REQUIRE(off + 2u <= bytes.size());
    return static_cast<std::uint16_t>(bytes[off])
         | (static_cast<std::uint16_t>(bytes[off + 1u]) << 8u);
}

std::uint32_t read_u32le(const std::vector<std::uint8_t>& bytes, std::size_t off) {
    REQUIRE(off + 4u <= bytes.size());
    return static_cast<std::uint32_t>(bytes[off])
         | (static_cast<std::uint32_t>(bytes[off + 1u]) << 8u)
         | (static_cast<std::uint32_t>(bytes[off + 2u]) << 16u)
         | (static_cast<std::uint32_t>(bytes[off + 3u]) << 24u);
}

std::uint64_t read_u64le(const std::vector<std::uint8_t>& bytes, std::size_t off) {
    REQUIRE(off + 8u <= bytes.size());
    std::uint64_t value = 0;
    for (std::size_t i = 0; i < 8u; ++i) {
        value |= static_cast<std::uint64_t>(bytes[off + i]) << (i * 8u);
    }
    return value;
}

std::vector<Stmt> all_op_variant_stmts() {
    return {
        {0u, Constant{0x1122'3344'5566'7788ULL, OpSize::I64}},
        {1u, LoadReg{Gpr::Rax, OpSize::I64}},
        {2u, LoadSegBase{SegmentReg::Fs}},
        {std::nullopt, StoreReg{Gpr::Rbx, 7u, OpSize::I32}},
        {3u, BinOp{BinOpKind::Add, 1u, 2u, OpSize::I64}},
        {4u, Extend{3u, OpSize::I8, OpSize::I64, true}},
        {5u, Truncate{4u, OpSize::I16}},
        {6u, Compare{CondCode::Slt, 5u, 6u, OpSize::I32}},
        {7u, Select{CondCode::Ne, 7u, 8u, OpSize::I64}},
        {8u, LoadMem{9u, OpSize::I64}},
        {std::nullopt, StoreMem{10u, 11u, OpSize::I32}},
        {9u, LoadMemTSO{12u, OpSize::I16}},
        {std::nullopt, StoreMemTSO{13u, 14u, OpSize::I8}},
        {std::nullopt, GuestPc{0x401000u}},
        {std::nullopt, Jump{2u}},
        {std::nullopt, CondJump{15u, 1u, 2u}},
        {std::nullopt, Return{}},
        {std::nullopt, JumpReg{16u}},
        {std::nullopt, CmpFlags{17u, 18u, OpSize::I64}},
        {std::nullopt, JumpRel{0x402000u}},
        {std::nullopt, CallRel{0x403000u, 0x403005u}},
        {std::nullopt, CallReg{19u, 0x404005u}},
        {std::nullopt, RetAdjusted{16u}},
        {std::nullopt, Cpuid{}},
        {std::nullopt, Syscall{}},
        {std::nullopt, Trap{TrapKind::Sigtrap}},
        {std::nullopt, Fence{FenceKind::Mfence}},
        {std::nullopt, CondJumpRel{CondCode::Eq, 0x405000u, 0x405010u}},
    };
}

}  // namespace

TEST_CASE("IR serialization: simple program has stable header and format") {
    const auto bytes = serialize_function(simple_program());

    REQUIRE(bytes.size() >= 16u);
    REQUIRE(std::equal(kIrBinaryMagic.begin(), kIrBinaryMagic.end(), bytes.begin()));

    REQUIRE(read_u16le(bytes, 4u) == kIrBinaryVersion);
    REQUIRE(bytes[6] == static_cast<std::uint8_t>(IrBinaryKind::Function));
    REQUIRE(bytes[7] == 0u);               // flags/reserved
    REQUIRE(read_u32le(bytes, 8u) == 0u);  // entry block id
    REQUIRE(read_u32le(bytes, 12u) == 1u); // block count

    REQUIRE(bytes == serialize_function(simple_program()));
}

TEST_CASE("IR serialization: every Op variant serializes deterministically") {
    for (const auto& stmt : all_op_variant_stmts()) {
        const std::array<Stmt, 1u> stmts{stmt};
        const auto first = serialize_stmts(stmts);
        const auto second = serialize_stmts(stmts);
        const auto op_first = serialize_op(stmt.op);
        const auto op_second = serialize_op(stmt.op);

        REQUIRE_FALSE(first.empty());
        REQUIRE_FALSE(op_first.empty());
        REQUIRE(first.size() >= 12u);
        REQUIRE(std::equal(kIrBinaryMagic.begin(), kIrBinaryMagic.end(), first.begin()));
        REQUIRE(read_u16le(first, 4u) == kIrBinaryVersion);
        REQUIRE(first[6] == static_cast<std::uint8_t>(IrBinaryKind::StmtList));
        REQUIRE(first == second);
        REQUIRE(op_first == op_second);
    }
}

TEST_CASE("IR serialization: Constant op bytes are little-endian and stable") {
    const auto bytes = serialize_op(Op{Constant{0x1122'3344'5566'7788ULL, OpSize::I64}});

    REQUIRE(bytes.size() == 10u);
    REQUIRE(bytes[0] == 1u);  // OpTag::Constant
    REQUIRE(read_u64le(bytes, 1u) == 0x1122'3344'5566'7788ULL);
    REQUIRE(bytes[9] == static_cast<std::uint8_t>(OpSize::I64));
}

TEST_CASE("IR serialization: function bytes preserve entry and block order") {
    Function fn;
    fn.entry = 7u;
    fn.blocks.push_back(BasicBlock{7u, {}});
    fn.blocks.push_back(BasicBlock{9u, {}});

    const auto bytes = serialize_function(fn);

    REQUIRE(bytes.size() >= 32u);
    REQUIRE(read_u32le(bytes, 8u) == 7u);
    REQUIRE(read_u32le(bytes, 12u) == 2u);

    // After the 16-byte function prefix, each block begins with its id followed by
    // the number of serialized statements. Empty blocks keep these offsets
    // independent from statement encoding details while pinning block order.
    REQUIRE(read_u32le(bytes, 16u) == 7u);
    REQUIRE(read_u32le(bytes, 20u) == 0u);
    REQUIRE(read_u32le(bytes, 24u) == 9u);
    REQUIRE(read_u32le(bytes, 28u) == 0u);
}
