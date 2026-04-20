// core/tests/test_passes_advanced.cpp — algebraic_simplify + CSE.
//
// Kept separate from test_passes.cpp so the two authors (claude and
// codex) can work on const_prop / dce and the newer advanced passes
// without stepping on each other's test files.

#include <catch2/catch_test_macros.hpp>
#include <variant>

#include "prisma/ir.hpp"
#include "prisma/passes.hpp"

using namespace prisma;

// ---------------------------------------------------------------------
// algebraic_simplify
// ---------------------------------------------------------------------

TEST_CASE("algebraic: x * 0 folds to const 0") {
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::Constant{0, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Mul, 0u, 1u, ir::OpSize::I64}},
    };
    auto out = passes::algebraic_simplify(s);
    REQUIRE(out.size() == 3);
    REQUIRE(std::holds_alternative<ir::Constant>(out[2].op));
    REQUIRE(std::get<ir::Constant>(out[2].op).value == 0u);
}

TEST_CASE("algebraic: 0 * x folds to const 0 (symmetry)") {
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{0, ir::OpSize::I64}},
        {1u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Mul, 0u, 1u, ir::OpSize::I64}},
    };
    auto out = passes::algebraic_simplify(s);
    REQUIRE(std::get<ir::Constant>(out[2].op).value == 0u);
}

TEST_CASE("algebraic: x & 0 folds to const 0") {
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rcx, ir::OpSize::I64}},
        {1u, ir::Constant{0, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::And, 0u, 1u, ir::OpSize::I64}},
    };
    auto out = passes::algebraic_simplify(s);
    REQUIRE(std::get<ir::Constant>(out[2].op).value == 0u);
}

TEST_CASE("algebraic: x | -1 folds to const -1 per size") {
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rcx, ir::OpSize::I32}},
        {1u, ir::Constant{0xFFFF'FFFFULL, ir::OpSize::I32}},
        {2u, ir::BinOp{ir::BinOpKind::Or, 0u, 1u, ir::OpSize::I32}},
    };
    auto out = passes::algebraic_simplify(s);
    REQUIRE(std::holds_alternative<ir::Constant>(out[2].op));
    REQUIRE(std::get<ir::Constant>(out[2].op).value == 0xFFFF'FFFFULL);
}

TEST_CASE("algebraic: x - x folds to const 0 (same ref)") {
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::BinOp{ir::BinOpKind::Sub, 0u, 0u, ir::OpSize::I64}},
    };
    auto out = passes::algebraic_simplify(s);
    REQUIRE(std::holds_alternative<ir::Constant>(out[1].op));
    REQUIRE(std::get<ir::Constant>(out[1].op).value == 0u);
}

TEST_CASE("algebraic: x ^ x folds to const 0 (zeroing idiom)") {
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::BinOp{ir::BinOpKind::Xor, 0u, 0u, ir::OpSize::I64}},
    };
    auto out = passes::algebraic_simplify(s);
    REQUIRE(std::get<ir::Constant>(out[1].op).value == 0u);
}

TEST_CASE("algebraic: x & y with y non-special is left alone") {
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::Constant{0x1234u, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::And, 0u, 1u, ir::OpSize::I64}},
    };
    auto out = passes::algebraic_simplify(s);
    REQUIRE(std::holds_alternative<ir::BinOp>(out[2].op));  // unchanged
}

TEST_CASE("algebraic: pipeline reduces `(x-x)+(y*0)` to `0 + 0`") {
    // A tiny program with two simplifiable BinOps — algebraic fires
    // on both; const_prop later would fold the sum. This test checks
    // just the algebraic pass in isolation.
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::BinOp{ir::BinOpKind::Sub, 0u, 0u, ir::OpSize::I64}},   // x - x → 0
        {2u, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}},
        {3u, ir::Constant{0, ir::OpSize::I64}},
        {4u, ir::BinOp{ir::BinOpKind::Mul, 2u, 3u, ir::OpSize::I64}},   // y * 0 → 0
    };
    auto out = passes::algebraic_simplify(s);
    REQUIRE(std::holds_alternative<ir::Constant>(out[1].op));
    REQUIRE(std::get<ir::Constant>(out[1].op).value == 0u);
    REQUIRE(std::holds_alternative<ir::Constant>(out[4].op));
    REQUIRE(std::get<ir::Constant>(out[4].op).value == 0u);
}

// ---------------------------------------------------------------------
// common_subexpression_eliminate
// ---------------------------------------------------------------------

TEST_CASE("cse: duplicate BinOp gets rewritten to a copy") {
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}},
        {3u, ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}},  // dup!
    };
    auto out = passes::common_subexpression_eliminate(s);
    REQUIRE(out.size() == 4);
    // First Add survives unchanged.
    REQUIRE(out[2].op == ir::Op{ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}});
    // Second Add rewritten to `Or, 2, 2` — our "copy from ref 2" form.
    REQUIRE(out[3].op == ir::Op{ir::BinOp{ir::BinOpKind::Or, 2u, 2u, ir::OpSize::I64}});
}

TEST_CASE("cse: distinct expressions are not merged") {
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}},
        {3u, ir::BinOp{ir::BinOpKind::Sub, 0u, 1u, ir::OpSize::I64}},  // different op
    };
    auto out = passes::common_subexpression_eliminate(s);
    REQUIRE(out[3].op == ir::Op{ir::BinOp{ir::BinOpKind::Sub, 0u, 1u, ir::OpSize::I64}});
}

TEST_CASE("cse: StoreReg conservatively flushes the table") {
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rcx, 2u, ir::OpSize::I64}},  // flush
        {3u, ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}},  // NOT deduped
    };
    auto out = passes::common_subexpression_eliminate(s);
    // The second Add survives as a real Add because StoreReg invalidated
    // the CSE table.
    REQUIRE(out[4].op == ir::Op{ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}});
}

TEST_CASE("cse: three duplicates all collapse to the first") {
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{1, ir::OpSize::I64}},
        {1u, ir::Constant{2, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}},
        {3u, ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}},
        {4u, ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}},
    };
    auto out = passes::common_subexpression_eliminate(s);
    // Every subsequent duplicate rewrites to `Or 2, 2` (copy from ref 2).
    REQUIRE(out[3].op == ir::Op{ir::BinOp{ir::BinOpKind::Or, 2u, 2u, ir::OpSize::I64}});
    REQUIRE(out[4].op == ir::Op{ir::BinOp{ir::BinOpKind::Or, 2u, 2u, ir::OpSize::I64}});
}

TEST_CASE("cse: idempotent") {
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Xor, 0u, 1u, ir::OpSize::I64}},
        {3u, ir::BinOp{ir::BinOpKind::Xor, 0u, 1u, ir::OpSize::I64}},
    };
    auto once  = passes::common_subexpression_eliminate(s);
    auto twice = passes::common_subexpression_eliminate(once);
    REQUIRE(once == twice);
}

// ---------------------------------------------------------------------
// default_pipeline integration
// ---------------------------------------------------------------------

TEST_CASE("pipeline: algebraic + const_prop collapse `(5+3)*0` fully") {
    // %0 = const 5
    // %1 = const 3
    // %2 = add %0, %1    → const 8 via const_prop (pass 1)
    // %3 = const 0
    // %4 = mul %2, %3    → 0 via algebraic (or const_prop since %2 is const)
    //      storereg rax, %4
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{5, ir::OpSize::I64}},
        {1u, ir::Constant{3, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}},
        {3u, ir::Constant{0, ir::OpSize::I64}},
        {4u, ir::BinOp{ir::BinOpKind::Mul, 2u, 3u, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 4u, ir::OpSize::I64}},
    };

    auto pm = passes::default_pipeline();
    auto [out, _stats] = pm.run(s);

    // After the full pipeline, the only surviving non-impure stmts
    // should be: `const 0` (ref 4), storereg, and possibly (ref 8 =
    // 5+3) but that one is dead and DCE'd.
    // The StoreReg value must be a ref that is a `const 0`.
    bool found_zero_store = false;
    for (const auto& s2 : out) {
        if (std::holds_alternative<ir::StoreReg>(s2.op)) {
            const auto& sr = std::get<ir::StoreReg>(s2.op);
            for (const auto& s3 : out) {
                if (s3.result && *s3.result == sr.value
                    && std::holds_alternative<ir::Constant>(s3.op)
                    && std::get<ir::Constant>(s3.op).value == 0u) {
                    found_zero_store = true;
                }
            }
        }
    }
    REQUIRE(found_zero_store);
}
