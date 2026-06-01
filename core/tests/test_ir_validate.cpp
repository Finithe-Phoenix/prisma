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

TEST_CASE("validate: BinOp rejects operand size mismatch") {
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{1, ir::OpSize::I64}},
        {1u, ir::Constant{2, ir::OpSize::I32}},
        {2u, ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}},
    };
    auto r = ir::validate(s);
    REQUIRE_FALSE(r.ok);
    REQUIRE(r.error.has_value());
    REQUIRE(r.error->code == ir::ValidationCode::SizeMismatch);
    REQUIRE(r.error->stmt_index == 2u);
}

TEST_CASE("validate: StoreReg rejects value too narrow for store size") {
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{1, ir::OpSize::I32}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 0u, ir::OpSize::I64}},
    };
    auto r = ir::validate(s);
    REQUIRE_FALSE(r.ok);
    REQUIRE(r.error.has_value());
    REQUIRE(r.error->code == ir::ValidationCode::SizeMismatch);
    REQUIRE(r.error->stmt_index == 1u);
}

TEST_CASE("validate: StoreReg permits storing low bits from wider value") {
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{1, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 0u, ir::OpSize::I16}},
    };
    REQUIRE(ir::validate(s).ok);
}

TEST_CASE("validate: StoreMem rejects value size mismatch") {
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::Constant{1, ir::OpSize::I32}},
        {std::nullopt, ir::StoreMem{0u, 1u, ir::OpSize::I64}},
    };
    auto r = ir::validate(s);
    REQUIRE_FALSE(r.ok);
    REQUIRE(r.error.has_value());
    REQUIRE(r.error->code == ir::ValidationCode::SizeMismatch);
    REQUIRE(r.error->stmt_index == 2u);
}

TEST_CASE("validate: Select with all operands defined passes") {
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{1, ir::OpSize::I64}},
        {1u, ir::Constant{2, ir::OpSize::I64}},
        {2u, ir::Select{ir::CondCode::Eq, 0u, 1u, ir::OpSize::I64}},
    };
    REQUIRE(ir::validate(s).ok);
}

TEST_CASE("validate: CmpFlags rejects operand size mismatch") {
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{1, ir::OpSize::I64}},
        {1u, ir::Constant{2, ir::OpSize::I32}},
        {std::nullopt, ir::CmpFlags{0u, 1u, ir::OpSize::I64}},
    };
    auto r = ir::validate(s);
    REQUIRE_FALSE(r.ok);
    REQUIRE(r.error.has_value());
    REQUIRE(r.error->code == ir::ValidationCode::SizeMismatch);
    REQUIRE(r.error->stmt_index == 2u);
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

TEST_CASE("validate: Extend rejects from_size mismatch") {
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{0xFF, ir::OpSize::I8}},
        {1u, ir::Extend{0u, ir::OpSize::I16, ir::OpSize::I64, false}},
    };
    auto r = ir::validate(s);
    REQUIRE_FALSE(r.ok);
    REQUIRE(r.error.has_value());
    REQUIRE(r.error->code == ir::ValidationCode::SizeMismatch);
    REQUIRE(r.error->stmt_index == 1u);
}

TEST_CASE("validate: Extend may follow an explicit Truncate from a wider source") {
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::Truncate{0u, ir::OpSize::I8}},
        {2u, ir::Extend{1u, ir::OpSize::I8, ir::OpSize::I16, true}},
    };
    REQUIRE(ir::validate(s).ok);
}

TEST_CASE("validate: LoadMem with I64 addr and matching value sizes passes") {
    std::vector<ir::Stmt> load_to_reg = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::LoadMem{0u, ir::OpSize::I32}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rbx, 1u, ir::OpSize::I32}},
    };
    REQUIRE(ir::validate(load_to_reg).ok);

    std::vector<ir::Stmt> load_to_mem = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::LoadMem{0u, ir::OpSize::I16}},
        {std::nullopt, ir::StoreMem{0u, 1u, ir::OpSize::I16}},
    };
    REQUIRE(ir::validate(load_to_mem).ok);
}

TEST_CASE("validate: LoadMem rejects non-I64 address") {
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I32}},
        {1u, ir::LoadMem{0u, ir::OpSize::I64}},
    };
    auto r = ir::validate(s);
    REQUIRE_FALSE(r.ok);
    REQUIRE(r.error.has_value());
    REQUIRE(r.error->code == ir::ValidationCode::SizeMismatch);
    REQUIRE(r.error->stmt_index == 1u);
}

TEST_CASE("validate: Compare boolean can be explicitly extended for i64 consumers") {
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{1, ir::OpSize::I64}},
        {1u, ir::Constant{2, ir::OpSize::I64}},
        {2u, ir::Compare{ir::CondCode::Eq, 0u, 1u, ir::OpSize::I64}},
        {3u, ir::Extend{2u, ir::OpSize::I8, ir::OpSize::I64, false}},
        {4u, ir::BinOp{ir::BinOpKind::And, 3u, 3u, ir::OpSize::I64}},
    };
    REQUIRE(ir::validate(s).ok);
}

TEST_CASE("validate: shift count ref may use a different integer size") {
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I16}},
        {1u, ir::Constant{15, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Sar, 0u, 1u, ir::OpSize::I16}},
    };
    REQUIRE(ir::validate(s).ok);
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

TEST_CASE("validate: GuestPc is a no-result marker") {
    std::vector<ir::Stmt> s = {
        {std::nullopt, ir::GuestPc{0x401000}},
        {0u, ir::Constant{1, ir::OpSize::I64}},
    };
    REQUIRE(ir::validate(s).ok);

    std::vector<ir::Stmt> bad = {
        {0u, ir::GuestPc{0x401000}},
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

// ---------------------------------------------------------------------
// F1-IR-015 typed-Ref consistency
// ---------------------------------------------------------------------

TEST_CASE("validate: BinOp with mismatched operand size is flagged") {
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{0xFF, ir::OpSize::I8}},          // i8
        {1u, ir::Constant{0xFFFF, ir::OpSize::I64}},       // i64
        {2u, ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}},
    };
    auto r = ir::validate(s);
    REQUIRE_FALSE(r.ok);
    REQUIRE(r.error->code == ir::ValidationCode::SizeMismatch);
    REQUIRE(r.error->bad_ref == 0u);
}

TEST_CASE("validate: Compare with matching i32 operands passes") {
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{0xCAFEBABE, ir::OpSize::I32}},
        {1u, ir::Constant{0xDEADBEEF, ir::OpSize::I32}},
        {2u, ir::Compare{ir::CondCode::Eq, 0u, 1u, ir::OpSize::I32}},
    };
    REQUIRE(ir::validate(s).ok);
}

TEST_CASE("validate: StoreReg with i32 value into i64 slot is flagged") {
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{0xCAFE, ir::OpSize::I32}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 0u, ir::OpSize::I64}},
    };
    auto r = ir::validate(s);
    REQUIRE_FALSE(r.ok);
    REQUIRE(r.error->code == ir::ValidationCode::SizeMismatch);
}

TEST_CASE("validate: StoreMem with non-I64 address is flagged") {
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{0x1000, ir::OpSize::I32}},
        {1u, ir::Constant{0xAA, ir::OpSize::I8}},
        {std::nullopt, ir::StoreMem{0u, 1u, ir::OpSize::I8}},
    };
    auto r = ir::validate(s);
    REQUIRE_FALSE(r.ok);
    REQUIRE(r.error->code == ir::ValidationCode::SizeMismatch);
    REQUIRE(r.error->bad_ref == 0u);
}

TEST_CASE("validate: Compare result is i8 (a bool); using it in i64 BinOp fails") {
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{1, ir::OpSize::I64}},
        {1u, ir::Constant{2, ir::OpSize::I64}},
        {2u, ir::Compare{ir::CondCode::Eq, 0u, 1u, ir::OpSize::I64}},
        {3u, ir::Constant{42, ir::OpSize::I64}},
        {4u, ir::BinOp{ir::BinOpKind::Add, 2u, 3u, ir::OpSize::I64}},
    };
    auto r = ir::validate(s);
    REQUIRE_FALSE(r.ok);
    REQUIRE(r.error->code == ir::ValidationCode::SizeMismatch);
}

TEST_CASE("validate: Extend declared from_size must agree with operand size") {
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{0xFF, ir::OpSize::I64}},
        {1u, ir::Extend{0u, ir::OpSize::I8, ir::OpSize::I64, /*signed=*/true}},
    };
    auto r = ir::validate(s);
    REQUIRE_FALSE(r.ok);
    REQUIRE(r.error->code == ir::ValidationCode::SizeMismatch);
}

TEST_CASE("validate: Select operands sharing the declared size passes") {
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{1, ir::OpSize::I32}},
        {1u, ir::Constant{2, ir::OpSize::I32}},
        {2u, ir::Select{ir::CondCode::Eq, 0u, 1u, ir::OpSize::I32}},
    };
    REQUIRE(ir::validate(s).ok);
}

TEST_CASE("validate: x87 stack ops require I64 payload refs") {
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{0x3FF0'0000'0000'0000ULL, ir::OpSize::I64}},
        {std::nullopt, ir::X87Push{0u}},
        {1u, ir::X87Load{0u}},
        {std::nullopt, ir::X87Store{1u, 1u}},
        {2u, ir::X87Pop{}},
    };
    REQUIRE(ir::validate(s).ok);
}

TEST_CASE("validate: x87 push with non-I64 value is rejected") {
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{1u, ir::OpSize::I32}},
        {std::nullopt, ir::X87Push{0u}},
    };
    auto r = ir::validate(s);
    REQUIRE_FALSE(r.ok);
    REQUIRE(r.error->code == ir::ValidationCode::SizeMismatch);
    REQUIRE(r.error->bad_ref == 0u);
}

TEST_CASE("validate: x87 store with undefined value is rejected") {
    std::vector<ir::Stmt> s = {
        {std::nullopt, ir::X87Store{0u, 99u}},
    };
    auto r = ir::validate(s);
    REQUIRE_FALSE(r.ok);
    REQUIRE(r.error->code == ir::ValidationCode::UndefinedRef);
    REQUIRE(r.error->bad_ref == 99u);
}

TEST_CASE("validate: AESKEYGENASSIST reads only its source vector") {
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadVecReg{1u}},
        {1u, ir::VecAesKeygenAssist{0u, 0x1Bu}},
        {std::nullopt, ir::StoreVecReg{2u, 1u}},
    };
    REQUIRE(ir::validate(s).ok);
}

TEST_CASE("validate: AESKEYGENASSIST undefined source is rejected") {
    std::vector<ir::Stmt> s = {
        {1u, ir::VecAesKeygenAssist{99u, 0x1Bu}},
    };
    auto r = ir::validate(s);
    REQUIRE_FALSE(r.ok);
    REQUIRE(r.error->code == ir::ValidationCode::UndefinedRef);
    REQUIRE(r.error->bad_ref == 99u);
}
