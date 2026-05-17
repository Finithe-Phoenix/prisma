// core/tests/test_ir_validate.cpp — F1-IR-016 validator coverage.

#include <catch2/catch_test_macros.hpp>

#include "prisma/ir.hpp"
#include "prisma/ir_validate.hpp"

using namespace prisma;

TEST_CASE("validate: empty stmt list is valid") {
    std::vector<ir::Stmt> s;
    auto r = ir::validate(s);
    REQUIRE(r.ok);
}

TEST_CASE("validate: simple well-formed program passes") {
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{1, ir::OpSize::I64}},
        {1u, ir::Constant{2, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}},
        {std::nullopt, ir::Return{}},
    };
    auto r = ir::validate(s);
    REQUIRE(r.ok);
}

TEST_CASE("validate: undefined ref is flagged") {
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        // BinOp references ref 42 which was never defined.
        {1u, ir::BinOp{ir::BinOpKind::Add, 0u, 42u, ir::OpSize::I64}},
    };
    auto r = ir::validate(s);
    REQUIRE_FALSE(r.ok);
    REQUIRE(r.error.has_value());
    REQUIRE(r.error->code == ir::ValidationCode::UndefinedRef);
    REQUIRE(r.error->stmt_index == 1u);
    REQUIRE(r.error->bad_ref == 42u);
}

TEST_CASE("validate: duplicate result ref is flagged") {
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{1, ir::OpSize::I64}},
        {0u, ir::Constant{2, ir::OpSize::I64}},  // same ref 0!
    };
    auto r = ir::validate(s);
    REQUIRE_FALSE(r.ok);
    REQUIRE(r.error->code == ir::ValidationCode::DuplicateResult);
    REQUIRE(r.error->stmt_index == 1u);
    REQUIRE(r.error->bad_ref == 0u);
}

TEST_CASE("validate: impure op with result ref is flagged") {
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{1, ir::OpSize::I64}},
        // StoreReg should NOT have a result ref.
        {7u, ir::StoreReg{ir::Gpr::Rax, 0u, ir::OpSize::I64}},
    };
    auto r = ir::validate(s);
    REQUIRE_FALSE(r.ok);
    REQUIRE(r.error->code == ir::ValidationCode::ImpureHasResult);
}

TEST_CASE("validate: pure op without result ref is flagged") {
    std::vector<ir::Stmt> s = {
        // Constant requires a result ref.
        {std::nullopt, ir::Constant{42, ir::OpSize::I64}},
    };
    auto r = ir::validate(s);
    REQUIRE_FALSE(r.ok);
    REQUIRE(r.error->code == ir::ValidationCode::PureLacksResult);
}

TEST_CASE("validate: StoreMem reads addr + value through the ref map") {
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},    // addr
        {1u, ir::Constant{42, ir::OpSize::I64}},             // value
        {std::nullopt, ir::StoreMem{0u, 1u, ir::OpSize::I64}},
    };
    REQUIRE(ir::validate(s).ok);
}

TEST_CASE("validate: StoreMem with undef value ref is flagged") {
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        // value ref 99 was never defined.
        {std::nullopt, ir::StoreMem{0u, 99u, ir::OpSize::I64}},
    };
    auto r = ir::validate(s);
    REQUIRE_FALSE(r.ok);
    REQUIRE(r.error->code == ir::ValidationCode::UndefinedRef);
    REQUIRE(r.error->bad_ref == 99u);
}

TEST_CASE("validate: Select with all operands defined passes") {
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{1, ir::OpSize::I64}},
        {1u, ir::Constant{2, ir::OpSize::I64}},
        {2u, ir::Select{ir::CondCode::Eq, 0u, 1u, ir::OpSize::I64}},
    };
    REQUIRE(ir::validate(s).ok);
}

TEST_CASE("validate: Extend and Truncate read their source ref") {
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{0xFF, ir::OpSize::I8}},
        {1u, ir::Extend{0u, ir::OpSize::I8, ir::OpSize::I64, true}},
        {2u, ir::Truncate{1u, ir::OpSize::I16}},
    };
    REQUIRE(ir::validate(s).ok);

    std::vector<ir::Stmt> bad = {
        {0u, ir::Truncate{99u, ir::OpSize::I16}},
    };
    auto r = ir::validate(bad);
    REQUIRE_FALSE(r.ok);
    REQUIRE(r.error->code == ir::ValidationCode::UndefinedRef);
    REQUIRE(r.error->bad_ref == 99u);
}

TEST_CASE("validate: Fence is side-effecting and has no result") {
    std::vector<ir::Stmt> s = {
        {std::nullopt, ir::Fence{ir::FenceKind::Mfence}},
    };
    REQUIRE(ir::validate(s).ok);

    std::vector<ir::Stmt> bad = {
        {0u, ir::Fence{ir::FenceKind::Sfence}},
    };
    auto r = ir::validate(bad);
    REQUIRE_FALSE(r.ok);
    REQUIRE(r.error->code == ir::ValidationCode::ImpureHasResult);
}

TEST_CASE("validate: forward self-reference is flagged as undefined") {
    // A ref that references itself before being defined.
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        // rhs references ref 5, which is this very stmt — but the def
        // isn't in `defs` yet at the moment of the check.
        {5u, ir::BinOp{ir::BinOpKind::Add, 0u, 5u, ir::OpSize::I64}},
    };
    auto r = ir::validate(s);
    REQUIRE_FALSE(r.ok);
    REQUIRE(r.error->code == ir::ValidationCode::UndefinedRef);
}
