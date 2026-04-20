// core/tests/test_passes.cpp — tests for IR optimisation passes.

#include <catch2/catch_test_macros.hpp>
#include <variant>

#include "prisma/ir.hpp"
#include "prisma/passes.hpp"

using namespace prisma;

TEST_CASE("const_prop: Add of two constants folds to one Constant") {
    std::vector<ir::Stmt> stmts = {
        {0u, ir::Constant{10, ir::OpSize::I64}},
        {1u, ir::Constant{32, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}},
        {std::nullopt, ir::Return{}},
    };

    auto out = passes::constant_propagate(stmts);

    // Same number of statements (we don't DCE the original Constants).
    REQUIRE(out.size() == stmts.size());

    // First two remain Constants.
    REQUIRE(std::holds_alternative<ir::Constant>(out[0].op));
    REQUIRE(std::holds_alternative<ir::Constant>(out[1].op));

    // Third is now a Constant of value 42.
    REQUIRE(std::holds_alternative<ir::Constant>(out[2].op));
    REQUIRE(out[2].result == std::optional<ir::Ref>(2));
    REQUIRE(std::get<ir::Constant>(out[2].op) ==
            ir::Constant{42, ir::OpSize::I64});

    // Return passes through.
    REQUIRE(std::holds_alternative<ir::Return>(out[3].op));
}

TEST_CASE("const_prop: Sub / And / Or / Xor all fold correctly") {
    auto fold_one = [](ir::BinOpKind op, std::uint64_t a, std::uint64_t b) {
        std::vector<ir::Stmt> s = {
            {0u, ir::Constant{a, ir::OpSize::I64}},
            {1u, ir::Constant{b, ir::OpSize::I64}},
            {2u, ir::BinOp{op, 0u, 1u, ir::OpSize::I64}},
        };
        auto out = passes::constant_propagate(s);
        REQUIRE(std::holds_alternative<ir::Constant>(out[2].op));
        return std::get<ir::Constant>(out[2].op).value;
    };

    REQUIRE(fold_one(ir::BinOpKind::Sub, 100, 58) == 42);
    REQUIRE(fold_one(ir::BinOpKind::And, 0xFF, 0x0F) == 0x0F);
    REQUIRE(fold_one(ir::BinOpKind::Or,  0xF0, 0x0F) == 0xFF);
    REQUIRE(fold_one(ir::BinOpKind::Xor, 0xFF, 0x0F) == 0xF0);
}

TEST_CASE("const_prop: result is size-masked (i32 addition overflow)") {
    // 0xFFFFFFFF + 1 in i32 wraps to 0. Our mask_to_size should clip it.
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{0xFFFF'FFFFULL, ir::OpSize::I32}},
        {1u, ir::Constant{1, ir::OpSize::I32}},
        {2u, ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I32}},
    };
    auto out = passes::constant_propagate(s);
    REQUIRE(std::get<ir::Constant>(out[2].op).value == 0u);
    REQUIRE(std::get<ir::Constant>(out[2].op).size  == ir::OpSize::I32);
}

TEST_CASE("const_prop: BinOp with a non-constant operand is NOT folded") {
    // %0 = loadreg rax   (unknown value)
    // %1 = const 7
    // %2 = add %0, %1    ← must stay as BinOp, no fold
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::Constant{7, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}},
    };
    auto out = passes::constant_propagate(s);
    REQUIRE(std::holds_alternative<ir::LoadReg>(out[0].op));
    REQUIRE(std::holds_alternative<ir::Constant>(out[1].op));
    REQUIRE(std::holds_alternative<ir::BinOp>(out[2].op));  // unchanged
}

TEST_CASE("const_prop: StoreReg + Return pass through untouched") {
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{99, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rbx, 0u, ir::OpSize::I64}},
        {std::nullopt, ir::Return{}},
    };
    auto out = passes::constant_propagate(s);
    REQUIRE(out.size() == s.size());
    REQUIRE(out[1].op == ir::Op{ir::StoreReg{ir::Gpr::Rbx, 0u, ir::OpSize::I64}});
    REQUIRE(out[2].op == ir::Op{ir::Return{}});
}

TEST_CASE("const_prop: idempotent — folding again is a no-op") {
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{5, ir::OpSize::I64}},
        {1u, ir::Constant{6, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}},
    };
    auto once = passes::constant_propagate(s);
    auto twice = passes::constant_propagate(once);
    REQUIRE(once == twice);
}

TEST_CASE("const_prop: transitive folding through a chain") {
    // %0 = 2
    // %1 = 3
    // %2 = add %0, %1      → should become const 5
    // %3 = 10
    // %4 = mul — we have sub instead:
    // %4 = sub %3, %2      → 10 - 5 = 5
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{2, ir::OpSize::I64}},
        {1u, ir::Constant{3, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}},
        {3u, ir::Constant{10, ir::OpSize::I64}},
        {4u, ir::BinOp{ir::BinOpKind::Sub, 3u, 2u, ir::OpSize::I64}},
    };
    auto out = passes::constant_propagate(s);
    REQUIRE(std::get<ir::Constant>(out[2].op).value == 5u);
    REQUIRE(std::get<ir::Constant>(out[4].op).value == 5u);
}
