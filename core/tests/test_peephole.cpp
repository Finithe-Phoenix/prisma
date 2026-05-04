// core/tests/test_peephole.cpp — F1-PS-009.

#include <catch2/catch_test_macros.hpp>

#include "prisma/ir.hpp"
#include "prisma/passes.hpp"

using namespace prisma;

TEST_CASE("peephole: empty input → empty output, no infinite loop") {
    auto out = passes::peephole_optimise_default({});
    REQUIRE(out.empty());
}

TEST_CASE("peephole: BinOp xor x, x folds to Constant 0 of the same size") {
    std::vector<ir::Stmt> in = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::BinOp{ir::BinOpKind::Xor, 0u, 0u, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rbx, 1u, ir::OpSize::I64}},
    };
    auto out = passes::peephole_optimise_default(in);
    REQUIRE(out.size() == 3);
    REQUIRE(std::holds_alternative<ir::Constant>(out[1].op));
    REQUIRE(std::get<ir::Constant>(out[1].op).value == 0u);
    REQUIRE(std::get<ir::Constant>(out[1].op).size == ir::OpSize::I64);
}

TEST_CASE("peephole: identity Extend (from_size == to_size) becomes Truncate") {
    std::vector<ir::Stmt> in = {
        {0u, ir::Constant{0xFF, ir::OpSize::I64}},
        {1u, ir::Extend{0u, ir::OpSize::I64, ir::OpSize::I64,
                        /*signed=*/true}},
    };
    auto out = passes::peephole_optimise_default(in);
    REQUIRE(out.size() == 2);
    REQUIRE(std::holds_alternative<ir::Truncate>(out[1].op));
    REQUIRE(std::get<ir::Truncate>(out[1].op).to_size == ir::OpSize::I64);
}

TEST_CASE("peephole: BinOp xor with different operands is left alone") {
    std::vector<ir::Stmt> in = {
        {0u, ir::Constant{1, ir::OpSize::I64}},
        {1u, ir::Constant{2, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Xor, 0u, 1u, ir::OpSize::I64}},
    };
    auto out = passes::peephole_optimise_default(in);
    REQUIRE(out.size() == 3);
    REQUIRE(std::holds_alternative<ir::BinOp>(out[2].op));
}

TEST_CASE("peephole: surrounding stmts pass through untouched") {
    std::vector<ir::Stmt> in = {
        {0u, ir::Constant{42, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 0u, ir::OpSize::I64}},
        {1u, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Xor, 1u, 1u, ir::OpSize::I64}},
        {std::nullopt, ir::Return{}},
    };
    auto out = passes::peephole_optimise_default(in);
    REQUIRE(out.size() == 5);
    REQUIRE(std::holds_alternative<ir::Constant>(out[0].op));
    REQUIRE(std::holds_alternative<ir::StoreReg>(out[1].op));
    REQUIRE(std::holds_alternative<ir::LoadReg>(out[2].op));
    REQUIRE(std::holds_alternative<ir::Constant>(out[3].op));  // xor folded
    REQUIRE(std::holds_alternative<ir::Return>(out[4].op));
}

TEST_CASE("peephole: idempotent (running twice is a fixed point)") {
    std::vector<ir::Stmt> in = {
        {0u, ir::LoadReg{ir::Gpr::Rcx, ir::OpSize::I64}},
        {1u, ir::BinOp{ir::BinOpKind::Xor, 0u, 0u, ir::OpSize::I64}},
    };
    auto a = passes::peephole_optimise_default(in);
    auto b = passes::peephole_optimise_default(a);
    REQUIRE(a == b);
}

TEST_CASE("peephole: BinOp or x, x folds to source via Truncate-identity") {
    std::vector<ir::Stmt> in = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::BinOp{ir::BinOpKind::Or, 0u, 0u, ir::OpSize::I64}},
    };
    auto out = passes::peephole_optimise_default(in);
    REQUIRE(std::holds_alternative<ir::Truncate>(out[1].op));
    REQUIRE(std::get<ir::Truncate>(out[1].op).value == 0u);
}

TEST_CASE("peephole: BinOp and x, x folds to source via Truncate-identity") {
    std::vector<ir::Stmt> in = {
        {0u, ir::LoadReg{ir::Gpr::Rcx, ir::OpSize::I64}},
        {1u, ir::BinOp{ir::BinOpKind::And, 0u, 0u, ir::OpSize::I64}},
    };
    auto out = passes::peephole_optimise_default(in);
    REQUIRE(std::holds_alternative<ir::Truncate>(out[1].op));
}

TEST_CASE("peephole: x + 0 folds to x") {
    std::vector<ir::Stmt> in = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::Constant{0, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}},
    };
    auto out = passes::peephole_optimise_default(in);
    REQUIRE(std::holds_alternative<ir::Truncate>(out[2].op));
    REQUIRE(std::get<ir::Truncate>(out[2].op).value == 0u);
}

TEST_CASE("peephole: 0 + x folds to x") {
    std::vector<ir::Stmt> in = {
        {0u, ir::Constant{0, ir::OpSize::I64}},
        {1u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}},
    };
    auto out = passes::peephole_optimise_default(in);
    REQUIRE(std::holds_alternative<ir::Truncate>(out[2].op));
    REQUIRE(std::get<ir::Truncate>(out[2].op).value == 1u);
}

TEST_CASE("peephole: x - 0 folds to x") {
    std::vector<ir::Stmt> in = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::Constant{0, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Sub, 0u, 1u, ir::OpSize::I64}},
    };
    auto out = passes::peephole_optimise_default(in);
    REQUIRE(std::holds_alternative<ir::Truncate>(out[2].op));
}

TEST_CASE("peephole: x * 1 folds to x; 1 * x folds to x") {
    std::vector<ir::Stmt> a = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::Constant{1, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Mul, 0u, 1u, ir::OpSize::I64}},
    };
    auto out_a = passes::peephole_optimise_default(a);
    REQUIRE(std::holds_alternative<ir::Truncate>(out_a[2].op));
    REQUIRE(std::get<ir::Truncate>(out_a[2].op).value == 0u);

    std::vector<ir::Stmt> b = {
        {0u, ir::Constant{1, ir::OpSize::I64}},
        {1u, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Mul, 0u, 1u, ir::OpSize::I64}},
    };
    auto out_b = passes::peephole_optimise_default(b);
    REQUIRE(std::holds_alternative<ir::Truncate>(out_b[2].op));
    REQUIRE(std::get<ir::Truncate>(out_b[2].op).value == 1u);
}

TEST_CASE("peephole: x * 0 folds to Constant 0") {
    std::vector<ir::Stmt> in = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::Constant{0, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Mul, 0u, 1u, ir::OpSize::I64}},
    };
    auto out = passes::peephole_optimise_default(in);
    REQUIRE(std::holds_alternative<ir::Constant>(out[2].op));
    REQUIRE(std::get<ir::Constant>(out[2].op).value == 0u);
}

TEST_CASE("peephole: Add of two non-zero non-constant operands is unchanged") {
    std::vector<ir::Stmt> in = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}},
    };
    auto out = passes::peephole_optimise_default(in);
    REQUIRE(std::holds_alternative<ir::BinOp>(out[2].op));
}

TEST_CASE("peephole: custom rule list (just XOR-self) does not touch Extend") {
    auto owned = passes::peephole_default_rules();
    // Use only the first rule (xor_self_to_zero).
    std::vector<const passes::PeepholeRule*> view{owned[0].get()};
    std::vector<ir::Stmt> in = {
        {0u, ir::Constant{0xFF, ir::OpSize::I8}},
        {1u, ir::Extend{0u, ir::OpSize::I8, ir::OpSize::I8, true}},  // identity
    };
    auto out = passes::peephole_optimise(in,
        std::span<const passes::PeepholeRule* const>(view));
    REQUIRE(std::holds_alternative<ir::Extend>(out[1].op));
}
