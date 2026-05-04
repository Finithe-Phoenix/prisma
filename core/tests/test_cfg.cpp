// core/tests/test_cfg.cpp — F1-IR-023 build_cfg.

#include <catch2/catch_test_macros.hpp>

#include "prisma/cfg.hpp"
#include "prisma/ir.hpp"

using namespace prisma;
using namespace prisma::ir;

TEST_CASE("build_cfg: empty input → empty Function") {
    auto fn = build_cfg(std::span<const Stmt>{});
    REQUIRE(fn.blocks.empty());
    REQUIRE(fn.entry == 0u);
}

TEST_CASE("build_cfg: single Return becomes a one-block function") {
    std::vector<Stmt> s = {{std::nullopt, Return{}}};
    auto fn = build_cfg(s);
    REQUIRE(fn.blocks.size() == 1);
    REQUIRE(fn.blocks[0].id == 0u);
    REQUIRE(fn.blocks[0].stmts.size() == 1);
}

TEST_CASE("build_cfg: stmts after a Return start a new block") {
    std::vector<Stmt> s = {
        {0u, Constant{42, OpSize::I64}},
        {std::nullopt, Return{}},
        {1u, Constant{99, OpSize::I64}},
        {std::nullopt, Return{}},
    };
    auto fn = build_cfg(s);
    REQUIRE(fn.blocks.size() == 2);
    REQUIRE(fn.blocks[0].id == 0u);
    REQUIRE(fn.blocks[0].stmts.size() == 2);
    REQUIRE(fn.blocks[1].id == 1u);
    REQUIRE(fn.blocks[1].stmts.size() == 2);
}

TEST_CASE("build_cfg: every terminator type ends a block") {
    // One terminator of each kind, separated by a no-op statement
    // (a constant that is never used). Should yield N+1 blocks
    // (the trailing one is empty so it's elided).
    std::vector<Stmt> s = {
        {0u, Constant{0, OpSize::I64}},  {std::nullopt, Return{}},
        {1u, Constant{1, OpSize::I64}},  {std::nullopt, JumpRel{0x1000ULL}},
        {2u, Constant{2, OpSize::I64}},
        {std::nullopt, CondJumpRel{CondCode::Eq, 0x100ULL, 0x200ULL}},
        {3u, Constant{3, OpSize::I64}},  {std::nullopt, Cpuid{}},
        {4u, Constant{4, OpSize::I64}},  {std::nullopt, Syscall{}},
        {5u, Constant{5, OpSize::I64}},  {std::nullopt, Trap{TrapKind::Sigtrap}},
        {6u, Constant{6, OpSize::I64}},
        {std::nullopt, CallRel{0x100ULL, 0x105ULL}},
        {7u, Constant{7, OpSize::I64}},  {std::nullopt, RetAdjusted{0u}},
        {8u, Constant{8, OpSize::I64}},  {std::nullopt, InlineAsm{{0xF4u}}},
    };
    auto fn = build_cfg(s);
    REQUIRE(fn.blocks.size() == 9);
    for (std::size_t i = 0; i < fn.blocks.size(); ++i) {
        REQUIRE(fn.blocks[i].id == i);
        REQUIRE(fn.blocks[i].stmts.size() == 2);
    }
}

TEST_CASE("build_cfg: trailing non-terminator stmts still get a block") {
    std::vector<Stmt> s = {
        {0u, Constant{42, OpSize::I64}},
        {std::nullopt, Return{}},
        {1u, Constant{99, OpSize::I64}},
        // No terminator at the end — decoder ran off the buffer.
    };
    auto fn = build_cfg(s);
    REQUIRE(fn.blocks.size() == 2);
    REQUIRE(fn.blocks[1].stmts.size() == 1);
}

TEST_CASE("is_terminator: all expected ops are terminators") {
    REQUIRE(is_terminator(Op{Return{}}));
    REQUIRE(is_terminator(Op{Jump{0u}}));
    REQUIRE(is_terminator(Op{CondJump{0u, 1u, 2u}}));
    REQUIRE(is_terminator(Op{JumpReg{0u}}));
    REQUIRE(is_terminator(Op{JumpRel{0x100ULL}}));
    REQUIRE(is_terminator(Op{CondJumpRel{CondCode::Eq, 0x100ULL, 0x200ULL}}));
    REQUIRE(is_terminator(Op{CallRel{0x100ULL, 0x105ULL}}));
    REQUIRE(is_terminator(Op{CallReg{0u, 0x105ULL}}));
    REQUIRE(is_terminator(Op{RetAdjusted{0u}}));
    REQUIRE(is_terminator(Op{Trap{TrapKind::Sigtrap}}));
    REQUIRE(is_terminator(Op{Cpuid{}}));
    REQUIRE(is_terminator(Op{Syscall{}}));
    REQUIRE(is_terminator(Op{InlineAsm{{}}}));
    REQUIRE(is_terminator(Op{CondJumpFlags{0u, CondCode::Eq, 1u, 2u}}));
}

TEST_CASE("is_terminator: pure ops are not terminators") {
    REQUIRE_FALSE(is_terminator(Op{Constant{0, OpSize::I64}}));
    REQUIRE_FALSE(is_terminator(Op{LoadReg{Gpr::Rax, OpSize::I64}}));
    REQUIRE_FALSE(is_terminator(Op{
        BinOp{BinOpKind::Add, 0u, 1u, OpSize::I64}}));
    REQUIRE_FALSE(is_terminator(Op{Fence{FenceKind::Mfence}}));
    REQUIRE_FALSE(is_terminator(Op{GuestPc{0x1000ULL}}));
    REQUIRE_FALSE(is_terminator(Op{
        Extend{0u, OpSize::I8, OpSize::I64, true}}));
}
