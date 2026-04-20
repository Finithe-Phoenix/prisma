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
    REQUIRE(fold_one(ir::BinOpKind::Mul, 6, 7) == 42);
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

// ---------------------------------------------------------------------------
// DCE pass.
// ---------------------------------------------------------------------------

TEST_CASE("dce: removes unused Constant") {
    // %0 = const 10
    // %1 = const 20   ← never read → dead
    // %2 = const 30
    //      storereg rax, %2
    //      ret
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{10, ir::OpSize::I64}},
        {1u, ir::Constant{20, ir::OpSize::I64}},
        {2u, ir::Constant{30, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}},
        {std::nullopt, ir::Return{}},
    };
    auto out = passes::dead_code_eliminate(s);

    // %0 also dead (never read); only %2, StoreReg, Return survive.
    REQUIRE(out.size() == 3);
    REQUIRE(out[0].op == ir::Op{ir::Constant{30, ir::OpSize::I64}});
    REQUIRE(out[1].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});
    REQUIRE(out[2].op == ir::Op{ir::Return{}});
}

TEST_CASE("dce: preserves live chain through BinOp") {
    // %0 = const 10
    // %1 = const 32
    // %2 = add %0, %1
    //      storereg rax, %2
    //      ret
    // All live.
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{10, ir::OpSize::I64}},
        {1u, ir::Constant{32, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}},
        {std::nullopt, ir::Return{}},
    };
    auto out = passes::dead_code_eliminate(s);
    REQUIRE(out.size() == s.size());
}

TEST_CASE("dce: const_prop leaves residue, dce cleans it up") {
    // Mirrors what the pipeline would do in Fase 1: after const_prop
    // replaces the BinOp with a Constant, the two original Constants
    // remain but become unused. DCE must remove them.
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{10, ir::OpSize::I64}},
        {1u, ir::Constant{32, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}},
        {std::nullopt, ir::Return{}},
    };

    auto after_cp  = passes::constant_propagate(s);
    auto after_dce = passes::dead_code_eliminate(after_cp);

    // after_cp: original %0, %1 survive (const_prop never deletes);
    //           %2 becomes `const 42`; store + return intact.
    REQUIRE(after_cp.size() == s.size());

    // after_dce: %0 and %1 are dead; only `const 42`, store, return remain.
    REQUIRE(after_dce.size() == 3);
    REQUIRE(after_dce[0].op == ir::Op{ir::Constant{42, ir::OpSize::I64}});
    REQUIRE(after_dce[1].op ==
            ir::Op{ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}});
    REQUIRE(after_dce[2].op == ir::Op{ir::Return{}});
}

TEST_CASE("dce: LoadReg is removed when its value is unused") {
    // %0 = loadreg rbx     ← dead
    // %1 = const 7
    //      storereg rax, %1
    //      ret
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}},
        {1u, ir::Constant{7, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 1u, ir::OpSize::I64}},
        {std::nullopt, ir::Return{}},
    };
    auto out = passes::dead_code_eliminate(s);
    REQUIRE(out.size() == 3);
    REQUIRE(std::holds_alternative<ir::Constant>(out[0].op));
}

TEST_CASE("dce: CmpFlags keeps its LoadReg operands alive") {
    // %0 = const 42
    //      storereg rax, %0
    // %1 = const 42
    //      storereg rbx, %1
    // %2 = loadreg rax
    // %3 = loadreg rbx
    //      cmpflags.i64 %2, %3
    //      condjmprel.eq ...
    //
    // This is important because `CmpFlags` has no SSA result, so DCE must
    // explicitly preserve the operand-producing statements it reads.
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{42, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 0u, ir::OpSize::I64}},
        {1u, ir::Constant{42, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rbx, 1u, ir::OpSize::I64}},
        {2u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {3u, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}},
        {std::nullopt, ir::CmpFlags{2u, 3u, ir::OpSize::I64}},
        {std::nullopt, ir::CondJumpRel{ir::CondCode::Eq, 0x1234, 0x5678}},
    };

    auto out = passes::dead_code_eliminate(s);
    REQUIRE(out.size() == 8);
    REQUIRE(std::get<ir::LoadReg>(out[4].op) == ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64});
    REQUIRE(std::get<ir::LoadReg>(out[5].op) == ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64});
    REQUIRE(std::holds_alternative<ir::CmpFlags>(out[6].op));
}

TEST_CASE("dce: StoreReg with dead Constant keeps the Constant (store is live)") {
    // %0 = const 99
    //      storereg rax, %0     ← impure, keeps %0 alive
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{99, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 0u, ir::OpSize::I64}},
    };
    auto out = passes::dead_code_eliminate(s);
    REQUIRE(out.size() == 2);
}

TEST_CASE("dce: LoadMemTSO is impure (conservatively kept)") {
    // %0 = loadreg rbx
    // %1 = load.tso [%0]   ← dead result, but TSO loads are observable
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}},
        {1u, ir::LoadMemTSO{0u, ir::OpSize::I64}},
    };
    auto out = passes::dead_code_eliminate(s);
    REQUIRE(out.size() == 2);  // both preserved
}

TEST_CASE("dce: plain LoadMem (non-TSO) IS removable when its result is dead") {
    // Contrast with the TSO test above: plain LoadMem is considered pure
    // for DCE, so an unused result deletes it.
    std::vector<ir::Stmt> s = {
        {0u, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}},
        {1u, ir::LoadMem{0u, ir::OpSize::I64}},  // dead
    };
    auto out = passes::dead_code_eliminate(s);
    REQUIRE(out.empty());
}

TEST_CASE("dce: idempotent") {
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{99, ir::OpSize::I64}},
        {1u, ir::Constant{55, ir::OpSize::I64}},  // dead
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 0u, ir::OpSize::I64}},
    };
    auto once  = passes::dead_code_eliminate(s);
    auto twice = passes::dead_code_eliminate(once);
    REQUIRE(once == twice);
}

// ---------------------------------------------------------------------------
// PassManager.
// ---------------------------------------------------------------------------

TEST_CASE("PassManager: default_pipeline runs const_prop then dce") {
    // Classic 10 + 32 → 42 flow.
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{10, ir::OpSize::I64}},
        {1u, ir::Constant{32, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}},
        {std::nullopt, ir::Return{}},
    };

    auto pm = passes::default_pipeline();
    REQUIRE(pm.size() == 2);

    auto [out, stats] = pm.run(s);

    // After the full pipeline: one Constant (the folded 42), one StoreReg,
    // one Return — the two source Constants are gone.
    REQUIRE(out.size() == 3);
    REQUIRE(std::get<ir::Constant>(out[0].op).value == 42u);

    REQUIRE(stats.initial_stmt_count == 5);
    REQUIRE(stats.passes.size() == 2);
    REQUIRE(stats.passes[0].name == "constant_propagate");
    REQUIRE(stats.passes[0].stmts_after == 5);  // const_prop keeps count
    REQUIRE(stats.passes[1].name == "dead_code_eliminate");
    REQUIRE(stats.passes[1].stmts_after == 3);  // DCE drops 2 dead defs
}

TEST_CASE("PassManager: empty pipeline is the identity") {
    passes::PassManager pm;
    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{7, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 0u, ir::OpSize::I64}},
    };
    auto [out, stats] = pm.run(s);
    REQUIRE(out == s);
    REQUIRE(stats.initial_stmt_count == s.size());
    REQUIRE(stats.passes.empty());
}

TEST_CASE("PassManager: order is insertion order") {
    // Register dce first (no-op on our input) then const_prop second.
    // The reverse of the default. Verifies ordering is respected.
    passes::PassManager pm;
    pm.add("dead_code_eliminate", passes::dead_code_eliminate)
      .add("constant_propagate",  passes::constant_propagate);

    std::vector<ir::Stmt> s = {
        {0u, ir::Constant{5, ir::OpSize::I64}},
        {1u, ir::Constant{6, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}},
    };

    auto [out, stats] = pm.run(s);

    // With DCE first (before folding), %0 and %1 are live because %2
    // still uses them. Nothing is removed. Then const_prop folds %2.
    REQUIRE(stats.passes[0].name == "dead_code_eliminate");
    REQUIRE(stats.passes[0].stmts_after == 4);
    REQUIRE(stats.passes[1].name == "constant_propagate");
    REQUIRE(stats.passes[1].stmts_after == 4);
    REQUIRE(std::get<ir::Constant>(out[2].op).value == 11u);
}

TEST_CASE("PassManager: never mutates its input") {
    std::vector<ir::Stmt> s_original = {
        {0u, ir::Constant{10, ir::OpSize::I64}},
        {1u, ir::Constant{32, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}},
    };
    auto s_copy_for_compare = s_original;

    auto pm = passes::default_pipeline();
    [[maybe_unused]] auto [_out, _stats] = pm.run(s_original);

    REQUIRE(s_original == s_copy_for_compare);
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
