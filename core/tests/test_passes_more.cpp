// core/tests/test_passes_more.cpp — unit tests for copy_propagate,
// strength_reduce, and branch_fold. Separate from test_passes_advanced
// so parallel work doesn't fight over one file.

#include <catch2/catch_test_macros.hpp>
#include <variant>

#include "prisma/ir.hpp"
#include "prisma/passes.hpp"

using namespace prisma;

// ---------------------------------------------------------------------
// copy_propagate
// ---------------------------------------------------------------------

TEST_CASE("copy_prop: `Or x, x` aliases subsequent uses of dst to x") {
    // %0 = loadreg rax
    // %1 = Or %0, %0           (CSE-style copy — dst alias → %0)
    // storereg rbx, %1         (should rewrite to storereg rbx, %0)
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::BinOp{ir::BinOpKind::Or, 0u, 0u, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rbx, 1u, ir::OpSize::I64}},
    };
    auto out = passes::copy_propagate(s);
    REQUIRE(out.size() == 3);
    REQUIRE(std::holds_alternative<ir::StoreReg>(out[2].op));
    REQUIRE(std::get<ir::StoreReg>(out[2].op).value == 0u);
}

TEST_CASE("copy_prop: does NOT alias on Or x, y with distinct operands") {
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Or, 0u, 1u, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rcx, 2u, ir::OpSize::I64}},
    };
    auto out = passes::copy_propagate(s);
    REQUIRE(std::get<ir::StoreReg>(out[3].op).value == 2u);  // untouched
}

TEST_CASE("copy_prop: chains of Or-copies collapse to the root ref") {
    // %0 = loadreg rax
    // %1 = Or %0, %0     → alias 1 → 0
    // %2 = Or %1, %1     → alias 2 → 0 (after 1 resolved to 0)
    // storereg rbx, %2   → rewritten to storereg rbx, %0
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::BinOp{ir::BinOpKind::Or, 0u, 0u, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Or, 1u, 1u, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rbx, 2u, ir::OpSize::I64}},
    };
    auto out = passes::copy_propagate(s);
    REQUIRE(std::get<ir::StoreReg>(out[3].op).value == 0u);
}

TEST_CASE("copy_prop: rewrites BinOp operands through the alias") {
    // %0 = loadreg rax
    // %1 = loadreg rbx
    // %2 = Or %1, %1          (alias 2 → 1)
    // %3 = Add %0, %2         (rewrites to Add %0, %1)
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Or, 1u, 1u, ir::OpSize::I64}},
        {3u, ir::BinOp{ir::BinOpKind::Add, 0u, 2u, ir::OpSize::I64}},
    };
    auto out = passes::copy_propagate(s);
    const auto& b = std::get<ir::BinOp>(out[3].op);
    REQUIRE(b.op == ir::BinOpKind::Add);
    REQUIRE(b.lhs == 0u);
    REQUIRE(b.rhs == 1u);  // was %2, now %1
}

TEST_CASE("copy_prop: idempotent") {
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::BinOp{ir::BinOpKind::Or, 0u, 0u, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rbx, 1u, ir::OpSize::I64}},
    };
    auto once  = passes::copy_propagate(s);
    auto twice = passes::copy_propagate(once);
    REQUIRE(once == twice);
}

// ---------------------------------------------------------------------
// strength_reduce
// ---------------------------------------------------------------------

TEST_CASE("strength_reduce: x * 8 → x << 3") {
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::Constant{8, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Mul, 0u, 1u, ir::OpSize::I64}},
    };
    auto out = passes::strength_reduce(s);
    // A fresh shift-count Constant must have been minted before %2's rewrite.
    // Find the rewritten Mul → it should now be Shl whose rhs is a Constant 3.
    // Locate the stmt whose result == 2.
    const ir::Stmt* mul = nullptr;
    for (const auto& st : out) if (st.result && *st.result == 2u) mul = &st;
    REQUIRE(mul);
    REQUIRE(std::holds_alternative<ir::BinOp>(mul->op));
    const auto& b = std::get<ir::BinOp>(mul->op);
    REQUIRE(b.op == ir::BinOpKind::Shl);
    REQUIRE(b.lhs == 0u);
    // rhs is a freshly-minted ref; find its Constant def.
    const ir::Stmt* k = nullptr;
    for (const auto& st : out) if (st.result && *st.result == b.rhs) k = &st;
    REQUIRE(k);
    REQUIRE(std::holds_alternative<ir::Constant>(k->op));
    REQUIRE(std::get<ir::Constant>(k->op).value == 3u);
}

TEST_CASE("strength_reduce: constant on the left also triggers (2 * x → x << 1)") {
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{2, ir::OpSize::I64}},
        {1u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Mul, 0u, 1u, ir::OpSize::I64}},
    };
    auto out = passes::strength_reduce(s);
    const ir::Stmt* mul = nullptr;
    for (const auto& st : out) if (st.result && *st.result == 2u) mul = &st;
    REQUIRE(mul);
    const auto& b = std::get<ir::BinOp>(mul->op);
    REQUIRE(b.op == ir::BinOpKind::Shl);
    REQUIRE(b.lhs == 1u);  // the non-constant side
}

TEST_CASE("strength_reduce: non-power-of-two leaves Mul alone") {
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::Constant{6, ir::OpSize::I64}},  // not 2^k
        {2u, ir::BinOp{ir::BinOpKind::Mul, 0u, 1u, ir::OpSize::I64}},
    };
    auto out = passes::strength_reduce(s);
    REQUIRE(out.size() == 3);
    REQUIRE(std::get<ir::BinOp>(out[2].op).op == ir::BinOpKind::Mul);
}

TEST_CASE("strength_reduce: Mul by 0 is left alone (algebraic handles it)") {
    // Zero is not a power of two by log2_pow2's definition, so this pass
    // should not touch it. algebraic_simplify collapses x*0 → 0 instead.
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::Constant{0, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Mul, 0u, 1u, ir::OpSize::I64}},
    };
    auto out = passes::strength_reduce(s);
    REQUIRE(std::get<ir::BinOp>(out[2].op).op == ir::BinOpKind::Mul);
}

TEST_CASE("strength_reduce: Add by power of two is NOT transformed") {
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::Constant{8, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}},
    };
    auto out = passes::strength_reduce(s);
    REQUIRE(std::get<ir::BinOp>(out[2].op).op == ir::BinOpKind::Add);
}

// ---------------------------------------------------------------------
// branch_fold
// ---------------------------------------------------------------------

TEST_CASE("branch_fold: Eq on equal constants takes the branch") {
    // cmp 5, 5
    // je target=0x100, ft=0x200
    //   → jmp 0x100
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{5, ir::OpSize::I64}},
        {1u, ir::Constant{5, ir::OpSize::I64}},
        {std::nullopt, ir::CmpFlags{0u, 1u, ir::OpSize::I64}},
        {std::nullopt,
         ir::CondJumpRel{ir::CondCode::Eq, /*target=*/0x100, /*ft=*/0x200}},
    };
    auto out = passes::branch_fold(s);
    // Find the jump stmt — it should now be an unconditional JumpRel(0x100).
    bool folded = false;
    for (const auto& st : out) {
        if (std::holds_alternative<ir::JumpRel>(st.op)) {
            REQUIRE(std::get<ir::JumpRel>(st.op).target_guest_pc == 0x100u);
            folded = true;
        }
    }
    REQUIRE(folded);
    // No CondJumpRel should survive.
    for (const auto& st : out) {
        REQUIRE_FALSE(std::holds_alternative<ir::CondJumpRel>(st.op));
    }
}

TEST_CASE("branch_fold: Ne on equal constants takes the fallthrough") {
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{7, ir::OpSize::I64}},
        {1u, ir::Constant{7, ir::OpSize::I64}},
        {std::nullopt, ir::CmpFlags{0u, 1u, ir::OpSize::I64}},
        {std::nullopt,
         ir::CondJumpRel{ir::CondCode::Ne, 0x100, 0x200}},
    };
    auto out = passes::branch_fold(s);
    for (const auto& st : out) {
        if (std::holds_alternative<ir::JumpRel>(st.op)) {
            REQUIRE(std::get<ir::JumpRel>(st.op).target_guest_pc == 0x200u);
        }
    }
}

TEST_CASE("branch_fold: signed Slt respects sign extension") {
    // i32-sized compare of 0xFFFFFFFF (-1 when sign-extended) vs 1:
    //   slt -1, 1 → true, so branch taken.
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{0xFFFFFFFFULL, ir::OpSize::I32}},
        {1u, ir::Constant{1, ir::OpSize::I32}},
        {std::nullopt, ir::CmpFlags{0u, 1u, ir::OpSize::I32}},
        {std::nullopt,
         ir::CondJumpRel{ir::CondCode::Slt, 0xAA, 0xBB}},
    };
    auto out = passes::branch_fold(s);
    bool found = false;
    for (const auto& st : out) {
        if (std::holds_alternative<ir::JumpRel>(st.op)) {
            REQUIRE(std::get<ir::JumpRel>(st.op).target_guest_pc == 0xAAu);
            found = true;
        }
    }
    REQUIRE(found);
}

TEST_CASE("branch_fold: unsigned Ult treats 0xFFFFFFFF as a large value") {
    // Ult 0xFFFFFFFF, 1 → false (0xFFFF... is huge unsigned), so fall through.
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{0xFFFFFFFFULL, ir::OpSize::I32}},
        {1u, ir::Constant{1, ir::OpSize::I32}},
        {std::nullopt, ir::CmpFlags{0u, 1u, ir::OpSize::I32}},
        {std::nullopt,
         ir::CondJumpRel{ir::CondCode::Ult, 0xAA, 0xBB}},
    };
    auto out = passes::branch_fold(s);
    for (const auto& st : out) {
        if (std::holds_alternative<ir::JumpRel>(st.op)) {
            REQUIRE(std::get<ir::JumpRel>(st.op).target_guest_pc == 0xBBu);
        }
    }
}

TEST_CASE("branch_fold: non-constant operands leave the branch alone") {
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::Constant{5, ir::OpSize::I64}},
        {std::nullopt, ir::CmpFlags{0u, 1u, ir::OpSize::I64}},
        {std::nullopt,
         ir::CondJumpRel{ir::CondCode::Eq, 0x100, 0x200}},
    };
    auto out = passes::branch_fold(s);
    bool kept_cond = false;
    for (const auto& st : out) {
        if (std::holds_alternative<ir::CondJumpRel>(st.op)) kept_cond = true;
    }
    REQUIRE(kept_cond);
}

TEST_CASE("branch_fold: flag-direct cc (Cc) is conservatively untouched") {
    // We can't decide Cc from plain integer compare — preserve the branch.
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{1, ir::OpSize::I64}},
        {1u, ir::Constant{1, ir::OpSize::I64}},
        {std::nullopt, ir::CmpFlags{0u, 1u, ir::OpSize::I64}},
        {std::nullopt,
         ir::CondJumpRel{ir::CondCode::Cc, 0x100, 0x200}},
    };
    auto out = passes::branch_fold(s);
    bool kept_cond = false;
    for (const auto& st : out) {
        REQUIRE_FALSE(std::holds_alternative<ir::JumpRel>(st.op));
        if (std::holds_alternative<ir::CondJumpRel>(st.op)) kept_cond = true;
    }
    REQUIRE(kept_cond);
}

// ---------------------------------------------------------------------
// flag_write_elimination
// ---------------------------------------------------------------------

TEST_CASE("flag_write_elimination: removes unused CmpFlags") {
    // cmpflags written here has no later CondJumpRel, so it can be dropped.
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{0x42, ir::OpSize::I64}},
        {1u, ir::Constant{0x43, ir::OpSize::I64}},
        {std::nullopt, ir::CmpFlags{0u, 1u, ir::OpSize::I64}},
        {std::nullopt, ir::Return{}},
    };
    auto out = passes::flag_write_elimination(s);
    REQUIRE(out.size() == 3);
    for (const auto& st : out) {
        REQUIRE_FALSE(std::holds_alternative<ir::CmpFlags>(st.op));
    }
}

TEST_CASE("flag_write_elimination: keeps CmpFlags required by CondJumpRel") {
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{9, ir::OpSize::I64}},
        {1u, ir::Constant{9, ir::OpSize::I64}},
        {std::nullopt, ir::CmpFlags{0u, 1u, ir::OpSize::I64}},
        {std::nullopt,
         ir::CondJumpRel{ir::CondCode::Eq, 0x100, 0x200}},
    };
    auto out = passes::flag_write_elimination(s);
    int kept_cmp = 0;
    int kept_cond = 0;
    for (const auto& st : out) {
        if (std::holds_alternative<ir::CmpFlags>(st.op)) ++kept_cmp;
        if (std::holds_alternative<ir::CondJumpRel>(st.op)) ++kept_cond;
    }
    REQUIRE(kept_cmp == 1);
    REQUIRE(kept_cond == 1);
}

TEST_CASE("flag_write_elimination: drops older CmpFlags when a newer one appears first") {
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{0x1111, ir::OpSize::I64}},
        {1u, ir::Constant{0x2222, ir::OpSize::I64}},
        {std::nullopt, ir::CmpFlags{0u, 1u, ir::OpSize::I64}},
        {2u, ir::Constant{0xAAAA, ir::OpSize::I64}},
        {std::nullopt, ir::CmpFlags{2u, 1u, ir::OpSize::I64}},
        {std::nullopt,
         ir::CondJumpRel{ir::CondCode::Eq, 0x100, 0x200}},
    };
    auto out = passes::flag_write_elimination(s);
    int kept_cmp = 0;
    for (const auto& st : out) {
        if (std::holds_alternative<ir::CmpFlags>(st.op)) {
            ++kept_cmp;
            REQUIRE(std::get<ir::CmpFlags>(st.op).lhs == 2u);
        }
    }
    REQUIRE(kept_cmp == 1);
}

TEST_CASE("flag_write_elimination: clears stale writes on Compare") {
    // Compare writes flags; it can satisfy CondJumpRel, so the previous
    // CmpFlags becomes stale and should be removed.
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{5, ir::OpSize::I64}},
        {1u, ir::Constant{6, ir::OpSize::I64}},
        {2u, ir::Constant{7, ir::OpSize::I64}},
        {3u, ir::Constant{8, ir::OpSize::I64}},
        {std::nullopt, ir::CmpFlags{0u, 1u, ir::OpSize::I64}},
        {4u, ir::LoadSegBase{ir::SegmentReg::Fs}},
        {std::nullopt, ir::Compare{ir::CondCode::Eq, 2u, 3u, ir::OpSize::I64}},
        {std::nullopt,
         ir::CondJumpRel{ir::CondCode::Eq, 0x100, 0x200}},
    };
    auto out = passes::flag_write_elimination(s);
    int kept_cmp = 0;
    for (const auto& st : out) {
        if (std::holds_alternative<ir::CmpFlags>(st.op)) ++kept_cmp;
    }
    REQUIRE(kept_cmp == 0);
}

// ---------------------------------------------------------------------
// Interaction with default_pipeline
// ---------------------------------------------------------------------

TEST_CASE("pipeline: `x * 4` through default pipeline becomes x << 2") {
    // %0 = loadreg rax
    // %1 = const 4
    // %2 = mul %0, %1
    // storereg rbx, %2
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::Constant{4, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Mul, 0u, 1u, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rbx, 2u, ir::OpSize::I64}},
    };
    auto pm = passes::default_pipeline();
    auto [out, _stats] = pm.run(s);

    // The surviving BinOp feeding the StoreReg must be a Shl by 2.
    const ir::Stmt* store = nullptr;
    for (const auto& st : out) {
        if (std::holds_alternative<ir::StoreReg>(st.op)) store = &st;
    }
    REQUIRE(store);
    const auto value_ref = std::get<ir::StoreReg>(store->op).value;
    const ir::Stmt* feeder = nullptr;
    for (const auto& st : out) {
        if (st.result && *st.result == value_ref) feeder = &st;
    }
    REQUIRE(feeder);
    REQUIRE(std::holds_alternative<ir::BinOp>(feeder->op));
    const auto& b = std::get<ir::BinOp>(feeder->op);
    REQUIRE(b.op == ir::BinOpKind::Shl);
}

TEST_CASE("pipeline: flag-write elimination removes dead CmpFlags after branch_fold") {
    // cmpflags before a const-foldable CondJumpRel should disappear from
    // the default pipeline.
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{1, ir::OpSize::I64}},
        {1u, ir::Constant{1, ir::OpSize::I64}},
        {std::nullopt, ir::CmpFlags{0u, 1u, ir::OpSize::I64}},
        {std::nullopt,
         ir::CondJumpRel{ir::CondCode::Eq, /*target=*/0x100, /*ft=*/0x200}},
    };

    auto pm = passes::default_pipeline();
    auto [out, _stats] = pm.run(s);

    REQUIRE(out.size() == 1);
    REQUIRE(std::holds_alternative<ir::JumpRel>(out[0].op));
    REQUIRE(std::get<ir::JumpRel>(out[0].op).target_guest_pc == 0x100u);
    REQUIRE_FALSE(std::holds_alternative<ir::CmpFlags>(out[0].op));
}

// ---------------------------------------------------------------------
// F1-PS-015 tail-call optimisation
// ---------------------------------------------------------------------

TEST_CASE("tco: CallRel + RetAdjusted{0} folds to JumpRel") {
    std::vector<ir::Stmt> s = {
        {std::nullopt, ir::CallRel{/*target=*/0x1000, /*ret=*/0x1005}},
        {std::nullopt, ir::RetAdjusted{0}},
    };
    auto out = passes::tail_call_optimise(s);
    REQUIRE(out.size() == 1);
    REQUIRE(std::holds_alternative<ir::JumpRel>(out[0].op));
    REQUIRE(std::get<ir::JumpRel>(out[0].op).target_guest_pc == 0x1000u);
}

TEST_CASE("tco: RetAdjusted with non-zero pop_bytes is left alone") {
    std::vector<ir::Stmt> s = {
        {std::nullopt, ir::CallRel{0x1000, 0x1005}},
        {std::nullopt, ir::RetAdjusted{8}},
    };
    auto out = passes::tail_call_optimise(s);
    REQUIRE(out.size() == 2);
    REQUIRE(std::holds_alternative<ir::CallRel>(out[0].op));
    REQUIRE(std::holds_alternative<ir::RetAdjusted>(out[1].op));
}

TEST_CASE("tco: CallReg (indirect) is NOT folded under MVP scope") {
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {std::nullopt, ir::CallReg{0u, 0x1005}},
        {std::nullopt, ir::RetAdjusted{0}},
    };
    auto out = passes::tail_call_optimise(s);
    REQUIRE(out.size() == 3);
    REQUIRE(std::holds_alternative<ir::CallReg>(out[1].op));
}

TEST_CASE("tco: CallRel without an immediately-following RetAdjusted is left alone") {
    std::vector<ir::Stmt> s = {
        {std::nullopt, ir::CallRel{0x1000, 0x1005}},
        {std::nullopt, ir::Jump{42u}},
    };
    auto out = passes::tail_call_optimise(s);
    REQUIRE(out.size() == 2);
    REQUIRE(std::holds_alternative<ir::CallRel>(out[0].op));
    REQUIRE(std::holds_alternative<ir::Jump>(out[1].op));
}

TEST_CASE("tco: two adjacent CallRel + RetAdjusted{0} pairs both fold") {
    std::vector<ir::Stmt> s = {
        {std::nullopt, ir::CallRel{0x100, 0x105}},
        {std::nullopt, ir::RetAdjusted{0}},
        {std::nullopt, ir::CallRel{0x200, 0x205}},
        {std::nullopt, ir::RetAdjusted{0}},
    };
    auto out = passes::tail_call_optimise(s);
    REQUIRE(out.size() == 2);
    REQUIRE(std::get<ir::JumpRel>(out[0].op).target_guest_pc == 0x100u);
    REQUIRE(std::get<ir::JumpRel>(out[1].op).target_guest_pc == 0x200u);
}

TEST_CASE("tco: surrounding stmts pass through unchanged") {
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{1, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 0u, ir::OpSize::I64}},
        {std::nullopt, ir::CallRel{0x1000, 0x1005}},
        {std::nullopt, ir::RetAdjusted{0}},
    };
    auto out = passes::tail_call_optimise(s);
    REQUIRE(out.size() == 3);
    REQUIRE(std::holds_alternative<ir::Constant>(out[0].op));
    REQUIRE(std::holds_alternative<ir::StoreReg>(out[1].op));
    REQUIRE(std::holds_alternative<ir::JumpRel>(out[2].op));
}

TEST_CASE("tco: idempotent (no changes on a folded program)") {
    std::vector<ir::Stmt> s = {
        {std::nullopt, ir::CallRel{0x100, 0x105}},
        {std::nullopt, ir::RetAdjusted{0}},
    };
    auto once  = passes::tail_call_optimise(s);
    auto twice = passes::tail_call_optimise(once);
    REQUIRE(once == twice);
}
