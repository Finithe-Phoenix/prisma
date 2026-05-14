// core/tests/test_global_cse.cpp — F2-PS-004 Global Common Subexpression
// Elimination unit tests.

#include <catch2/catch_test_macros.hpp>

#include "prisma/ir.hpp"
#include "prisma/passes.hpp"

using namespace prisma;
using namespace prisma::ir;

namespace {

// Convenience: produce a `BinOp` op (Add of two refs at I64).
Op add_i64(Ref l, Ref r) { return Op{BinOp{BinOpKind::Add, l, r, OpSize::I64}}; }

// Check that a stmt's BinOp was rewritten to the canonical "Or x,x"
// copy idiom that intra-block CSE uses. Returns the source ref.
Ref expect_copy(const Stmt& s) {
    REQUIRE(std::holds_alternative<BinOp>(s.op));
    const auto& b = std::get<BinOp>(s.op);
    REQUIRE(b.op == BinOpKind::Or);
    REQUIRE(b.lhs == b.rhs);
    return b.lhs;
}

bool is_add(const Stmt& s) {
    if (!std::holds_alternative<BinOp>(s.op)) return false;
    return std::get<BinOp>(s.op).op == BinOpKind::Add;
}

}  // namespace

TEST_CASE("global_cse: single-block matches intra-block CSE (Add + Add same operands)") {
    Function fn;
    fn.entry = 0;
    fn.blocks.push_back(BasicBlock{0u, {
        {0u, Constant{10, OpSize::I64}},
        {1u, Constant{20, OpSize::I64}},
        {2u, add_i64(0u, 1u)},     // %2 = 10 + 20
        {3u, add_i64(0u, 1u)},     // %3 = 10 + 20 — redundant
        {std::nullopt, StoreReg{Gpr::Rax, 3u, OpSize::I64}},
        {std::nullopt, Return{}},
    }});

    const auto out = passes::global_cse(fn);
    REQUIRE(out.blocks.size() == 1);
    const auto& bb = out.blocks[0];
    REQUIRE(bb.stmts.size() == 6);
    REQUIRE(is_add(bb.stmts[2]));            // first Add untouched
    REQUIRE(expect_copy(bb.stmts[3]) == 2u); // second rewritten to copy of %2
}

TEST_CASE("global_cse: linear chain A→B forwards available expression") {
    // bb0: %0 = const 1; %1 = const 2; %2 = %0 + %1; Jump bb1
    // bb1: %3 = %0 + %1  ← same key as %2; should rewrite to Or(%2,%2)
    //      StoreReg rax, %3
    //      Return
    Function fn;
    fn.entry = 0;
    fn.blocks.push_back(BasicBlock{0u, {
        {0u, Constant{1, OpSize::I64}},
        {1u, Constant{2, OpSize::I64}},
        {2u, add_i64(0u, 1u)},
        {std::nullopt, Jump{1u}},
    }});
    fn.blocks.push_back(BasicBlock{1u, {
        {3u, add_i64(0u, 1u)},
        {std::nullopt, StoreReg{Gpr::Rax, 3u, OpSize::I64}},
        {std::nullopt, Return{}},
    }});

    const auto out = passes::global_cse(fn);
    REQUIRE(out.blocks.size() == 2);
    const auto& bb1 = out.blocks[1];
    REQUIRE(expect_copy(bb1.stmts[0]) == 2u);
}

TEST_CASE("global_cse: diamond join — no rewrite at the join block (conservative)") {
    // bb0: %0 = c1; %1 = c2; %2 = %0 + %1; CondJump (%0, bb1, bb2)
    // bb1: Jump bb3
    // bb2: Jump bb3
    // bb3: %3 = %0 + %1  ← matches %2 but bb3 has two predecessors,
    //                      so the MVP conservatively does NOT rewrite.
    //      StoreReg rax, %3; Return
    Function fn;
    fn.entry = 0;
    fn.blocks.push_back(BasicBlock{0u, {
        {0u, Constant{1, OpSize::I64}},
        {1u, Constant{2, OpSize::I64}},
        {2u, add_i64(0u, 1u)},
        {std::nullopt, CondJump{0u, 1u, 2u}},
    }});
    fn.blocks.push_back(BasicBlock{1u, {{std::nullopt, Jump{3u}}}});
    fn.blocks.push_back(BasicBlock{2u, {{std::nullopt, Jump{3u}}}});
    fn.blocks.push_back(BasicBlock{3u, {
        {3u, add_i64(0u, 1u)},
        {std::nullopt, StoreReg{Gpr::Rax, 3u, OpSize::I64}},
        {std::nullopt, Return{}},
    }});

    const auto out = passes::global_cse(fn);
    REQUIRE(out.blocks.size() == 4);
    const auto& bb3 = out.blocks[3];
    // The Add stays as Add (NOT rewritten to Or-copy).
    REQUIRE(is_add(bb3.stmts[0]));
    const auto& b = std::get<BinOp>(bb3.stmts[0].op);
    REQUIRE(b.op == BinOpKind::Add);   // explicitly NOT Or
}

TEST_CASE("global_cse: flushing op in predecessor invalidates forwarded table") {
    // bb0: %0 = c1; %1 = c2; %2 = %0 + %1; StoreReg rbx, %2; Jump bb1
    //                                       ^^^^^^^^^^^^^^^ flushes
    // bb1: %3 = %0 + %1 ← table cleared at the StoreReg in bb0, so no
    //                     entry survives to bb1; %3 stays as Add.
    Function fn;
    fn.entry = 0;
    fn.blocks.push_back(BasicBlock{0u, {
        {0u, Constant{1, OpSize::I64}},
        {1u, Constant{2, OpSize::I64}},
        {2u, add_i64(0u, 1u)},
        {std::nullopt, StoreReg{Gpr::Rbx, 2u, OpSize::I64}},
        {std::nullopt, Jump{1u}},
    }});
    fn.blocks.push_back(BasicBlock{1u, {
        {3u, add_i64(0u, 1u)},
        {std::nullopt, StoreReg{Gpr::Rax, 3u, OpSize::I64}},
        {std::nullopt, Return{}},
    }});

    const auto out = passes::global_cse(fn);
    const auto& bb1 = out.blocks[1];
    REQUIRE(is_add(bb1.stmts[0]));
    REQUIRE(std::get<BinOp>(bb1.stmts[0].op).op == BinOpKind::Add);
}

TEST_CASE("global_cse: FunctionPassManager runs the pipeline end-to-end") {
    Function fn;
    fn.entry = 0;
    fn.blocks.push_back(BasicBlock{0u, {
        {0u, Constant{10, OpSize::I64}},
        {1u, Constant{20, OpSize::I64}},
        {2u, add_i64(0u, 1u)},
        {3u, add_i64(0u, 1u)},
        {std::nullopt, Return{}},
    }});
    const auto pm = passes::default_function_pipeline();
    REQUIRE(pm.size() == 1);
    const auto [out, stats] = pm.run(fn);
    REQUIRE(stats.initial_block_count == 1);
    REQUIRE(stats.initial_stmt_count  == 5);
    REQUIRE(stats.passes.size() == 1);
    REQUIRE(stats.passes[0].name == "global_cse");
    REQUIRE(expect_copy(out.blocks[0].stmts[3]) == 2u);
}

TEST_CASE("global_cse: empty function is a no-op") {
    Function fn;  // no blocks, entry=0
    const auto out = passes::global_cse(fn);
    REQUIRE(out.blocks.empty());
    REQUIRE(out.entry == 0u);
}
