// core/tests/test_ir_cfg.cpp - flat IR to basic-block builder tests.

#include <catch2/catch_test_macros.hpp>

#include "prisma/ir.hpp"
#include "prisma/ir_cfg.hpp"

#include <optional>
#include <variant>
#include <vector>

using namespace prisma::ir;

TEST_CASE("IR CFG: linear statement list becomes one entry block") {
    const std::vector<Stmt> stmts{
        {0u, Constant{10u, OpSize::I64}},
        {std::nullopt, StoreReg{Gpr::Rax, 0u, OpSize::I64}},
    };

    const auto cfg = build_cfg(stmts);

    REQUIRE(cfg.ok);
    REQUIRE(cfg.function.entry == 0u);
    REQUIRE(cfg.function.blocks.size() == 1u);
    REQUIRE(cfg.function.blocks[0].id == 0u);
    REQUIRE(cfg.function.blocks[0].stmts == stmts);
    REQUIRE(flatten(cfg.function) == stmts);
}

TEST_CASE("IR CFG: terminators start the following statement in a new block") {
    const std::vector<Stmt> stmts{
        {std::nullopt, GuestPc{0x1000u}},
        {std::nullopt, JumpRel{0x2000u}},
        {std::nullopt, GuestPc{0x2000u}},
        {std::nullopt, Return{}},
        {std::nullopt, GuestPc{0x3000u}},
        {std::nullopt, Trap{TrapKind::Sigtrap}},
    };

    const auto cfg = build_cfg(stmts);

    REQUIRE(cfg.ok);
    REQUIRE(cfg.function.blocks.size() == 3u);
    REQUIRE(cfg.function.blocks[0].id == 0u);
    REQUIRE(cfg.function.blocks[0].stmts.size() == 2u);
    REQUIRE(cfg.function.blocks[0].stmts.back().op == Op{JumpRel{0x2000u}});
    REQUIRE(cfg.function.blocks[1].id == 1u);
    REQUIRE(cfg.function.blocks[1].stmts.front().op == Op{GuestPc{0x2000u}});
    REQUIRE(cfg.function.blocks[1].stmts.back().op == Op{Return{}});
    REQUIRE(cfg.function.blocks[2].id == 2u);
    REQUIRE(cfg.function.blocks[2].stmts.back().op == Op{Trap{TrapKind::Sigtrap}});
    REQUIRE(flatten(cfg.function) == stmts);
}

TEST_CASE("IR CFG: CondJumpRel guest targets become block boundaries") {
    const std::vector<Stmt> stmts{
        {std::nullopt, GuestPc{0x1000u}},
        {0u, Constant{1u, OpSize::I64}},
        {1u, Constant{2u, OpSize::I64}},
        {std::nullopt, CmpFlags{0u, 1u, OpSize::I64}},
        {std::nullopt, CondJumpRel{CondCode::Eq, 0x3000u, 0x2000u}},
        {std::nullopt, GuestPc{0x2000u}},
        {std::nullopt, Return{}},
        {std::nullopt, GuestPc{0x3000u}},
        {std::nullopt, Return{}},
    };

    const auto cfg = build_cfg(stmts);

    REQUIRE(cfg.ok);
    REQUIRE(cfg.function.blocks.size() == 3u);
    REQUIRE(cfg.function.blocks[0].stmts.size() == 5u);
    REQUIRE(cfg.function.blocks[0].stmts.back().op
            == Op{CondJumpRel{CondCode::Eq, 0x3000u, 0x2000u}});
    REQUIRE(cfg.function.blocks[1].stmts.front().op == Op{GuestPc{0x2000u}});
    REQUIRE(cfg.function.blocks[2].stmts.front().op == Op{GuestPc{0x3000u}});
    REQUIRE(flatten(cfg.function) == stmts);
}

TEST_CASE("IR CFG: unresolved JumpRel target is treated as external") {
    const std::vector<Stmt> stmts{
        {std::nullopt, GuestPc{0x1000u}},
        {std::nullopt, JumpRel{0x4000u}},
    };

    const auto cfg = build_cfg(stmts);

    REQUIRE(cfg.ok);
    REQUIRE(cfg.function.blocks.size() == 1u);
    REQUIRE(cfg.function.blocks[0].stmts == stmts);
}

TEST_CASE("IR CFG: backward guest target splits its destination block") {
    const std::vector<Stmt> stmts{
        {std::nullopt, GuestPc{0x1000u}},
        {0u, Constant{7u, OpSize::I64}},
        {std::nullopt, GuestPc{0x1004u}},
        {std::nullopt, StoreReg{Gpr::Rax, 0u, OpSize::I64}},
        {std::nullopt, JumpRel{0x1004u}},
    };

    const auto cfg = build_cfg(stmts);

    REQUIRE(cfg.ok);
    REQUIRE(cfg.function.blocks.size() == 2u);
    REQUIRE(cfg.function.blocks[0].stmts.size() == 2u);
    REQUIRE(cfg.function.blocks[1].stmts.front().op == Op{GuestPc{0x1004u}});
    REQUIRE(cfg.function.blocks[1].stmts.back().op == Op{JumpRel{0x1004u}});
    REQUIRE(flatten(cfg.function) == stmts);
}

TEST_CASE("IR CFG: direct block-indexed branches are terminators") {
    const std::vector<Stmt> stmts{
        {0u, Constant{1u, OpSize::I64}},
        {std::nullopt, CondJump{0u, 2u, 1u}},
        {std::nullopt, CondJumpFlags{0u, CondCode::Eq, 2u, 1u}},
        {std::nullopt, Jump{3u}},
        {std::nullopt, Return{}},
    };

    const auto cfg = build_cfg(stmts);

    REQUIRE(cfg.ok);
    REQUIRE(cfg.function.blocks.size() == 4u);
    REQUIRE(cfg.function.blocks[0].stmts.back().op == Op{CondJump{0u, 2u, 1u}});
    REQUIRE(cfg.function.blocks[1].stmts.back().op == Op{CondJumpFlags{0u, CondCode::Eq, 2u, 1u}});
    REQUIRE(cfg.function.blocks[2].stmts.back().op == Op{Jump{3u}});
    REQUIRE(cfg.function.blocks[3].stmts.back().op == Op{Return{}});
    REQUIRE(flatten(cfg.function) == stmts);
}

TEST_CASE("IR CFG: flatten emits blocks by block id") {
    Function function;
    function.entry = 1u;
    function.blocks.push_back(BasicBlock{2u, {{std::nullopt, Return{}}}});
    function.blocks.push_back(BasicBlock{0u, {{std::nullopt, GuestPc{0x1000u}}}});
    function.blocks.push_back(BasicBlock{1u, {{std::nullopt, Jump{2u}}}});

    const auto flat = flatten(function);

    REQUIRE(flat.size() == 3u);
    REQUIRE(flat[0].op == Op{GuestPc{0x1000u}});
    REQUIRE(flat[1].op == Op{Jump{2u}});
    REQUIRE(flat[2].op == Op{Return{}});
}

TEST_CASE("IR CFG: empty input is rejected") {
    const std::vector<Stmt> stmts;

    const auto cfg = build_cfg(stmts);

    REQUIRE_FALSE(cfg.ok);
    REQUIRE(cfg.error);
    REQUIRE(cfg.error->code == CfgBuildCode::EmptyInput);
}

TEST_CASE("IR CFG: duplicate GuestPc markers are rejected") {
    const std::vector<Stmt> stmts{
        {std::nullopt, GuestPc{0x1000u}},
        {std::nullopt, GuestPc{0x1000u}},
        {std::nullopt, Return{}},
    };

    const auto cfg = build_cfg(stmts);

    REQUIRE_FALSE(cfg.ok);
    REQUIRE(cfg.error);
    REQUIRE(cfg.error->code == CfgBuildCode::DuplicateGuestPc);
    REQUIRE(cfg.error->stmt_index == 1u);
}
