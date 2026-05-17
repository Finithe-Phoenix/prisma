// core/tests/test_ir.cpp — unit tests for the Prisma IR data structures.
//
// These tests validate structural equality, pretty-printing, and basic
// invariants. They mirror the toy proofs in `ir-spec/PrismaIR/Semantics.lean`
// so the C++ runtime and the Lean spec agree on ground-truth examples.

#include <catch2/catch_test_macros.hpp>

#include "prisma/ir.hpp"

using namespace prisma::ir;

TEST_CASE("OpSize bit_width and mask_to_size agree") {
    REQUIRE(bit_width(OpSize::I8)  == 8);
    REQUIRE(bit_width(OpSize::I16) == 16);
    REQUIRE(bit_width(OpSize::I32) == 32);
    REQUIRE(bit_width(OpSize::I64) == 64);

    REQUIRE(mask_to_size(0xFFFF'FFFF'FFFF'FFFFULL, OpSize::I8)  == 0xFFULL);
    REQUIRE(mask_to_size(0xFFFF'FFFF'FFFF'FFFFULL, OpSize::I16) == 0xFFFFULL);
    REQUIRE(mask_to_size(0xFFFF'FFFF'FFFF'FFFFULL, OpSize::I32) == 0xFFFF'FFFFULL);
    REQUIRE(mask_to_size(0xFFFF'FFFF'FFFF'FFFFULL, OpSize::I64) == 0xFFFF'FFFF'FFFF'FFFFULL);
}

TEST_CASE("Structural equality on Op variants") {
    Op a = Constant{42, OpSize::I64};
    Op b = Constant{42, OpSize::I64};
    Op c = Constant{42, OpSize::I32};  // different size
    Op d = Constant{43, OpSize::I64};  // different value

    REQUIRE(a == b);
    REQUIRE_FALSE(a == c);
    REQUIRE_FALSE(a == d);

    Op e = BinOp{BinOpKind::Add, 0, 1, OpSize::I64};
    Op f = BinOp{BinOpKind::Add, 0, 1, OpSize::I64};
    Op g = BinOp{BinOpKind::Sub, 0, 1, OpSize::I64};  // different op

    REQUIRE(e == f);
    REQUIRE_FALSE(e == g);

    Op h = Extend{0, OpSize::I8, OpSize::I64, true};
    Op i = Extend{0, OpSize::I8, OpSize::I64, true};
    Op j = Extend{0, OpSize::I8, OpSize::I64, false};
    Op k = Truncate{0, OpSize::I16};
    Op l = Truncate{0, OpSize::I32};

    REQUIRE(h == i);
    REQUIRE_FALSE(h == j);
    REQUIRE_FALSE(k == l);
}

TEST_CASE("Stmt equality includes result binding") {
    Stmt a{/*result=*/0, Constant{42, OpSize::I64}};
    Stmt b{/*result=*/0, Constant{42, OpSize::I64}};
    Stmt c{/*result=*/1, Constant{42, OpSize::I64}};  // different ref
    Stmt d{/*result=*/std::nullopt, Constant{42, OpSize::I64}};  // no result

    REQUIRE(a == b);
    REQUIRE_FALSE(a == c);
    REQUIRE_FALSE(a == d);
}

TEST_CASE("Lean exampleProgram mirrored in C++") {
    // This is the same example as in ir-spec/PrismaIR/Semantics.lean
    // `exampleProgram`: bb0 { %0=const 10; %1=const 32; %2=add %0, %1; ret }
    Function fn;
    fn.entry = 0;

    BasicBlock bb0;
    bb0.id = 0;
    bb0.stmts = {
        {/*result=*/0u, Constant{10, OpSize::I64}},
        {/*result=*/1u, Constant{32, OpSize::I64}},
        {/*result=*/2u, BinOp{BinOpKind::Add, 0, 1, OpSize::I64}},
        {/*result=*/std::nullopt, Return{}},
    };
    fn.blocks.push_back(bb0);

    REQUIRE(fn.blocks.size() == 1);
    REQUIRE(fn.blocks[0].stmts.size() == 4);
    REQUIRE(fn.blocks[0].stmts.back().op == Op{Return{}});
}

TEST_CASE("Pretty-print produces stable-looking output for the example") {
    Stmt s_const{0u, Constant{42, OpSize::I64}};
    REQUIRE(pretty_print(s_const) == "%0 = const.i64 0x2a");

    Stmt s_add{2u, BinOp{BinOpKind::Add, 0u, 1u, OpSize::I64}};
    REQUIRE(pretty_print(s_add) == "%2 = add.i64 %0, %1");

    Stmt s_sext{3u, Extend{2u, OpSize::I8, OpSize::I64, true}};
    REQUIRE(pretty_print(s_sext) == "%3 = sext.i8->i64 %2");

    Stmt s_trunc{4u, Truncate{3u, OpSize::I16}};
    REQUIRE(pretty_print(s_trunc) == "%4 = trunc.i16 %3");

    Stmt s_ret{std::nullopt, Return{}};
    REQUIRE(pretty_print(s_ret) == "ret");

    Stmt s_store_tso{std::nullopt, StoreMemTSO{/*addr=*/5u, /*value=*/6u, OpSize::I32}};
    REQUIRE(pretty_print(s_store_tso) == "store.tso.i32 [%5], %6");
}

TEST_CASE("kInvalidRef renders as %?") {
    Stmt s{kInvalidRef, Constant{0, OpSize::I8}};
    REQUIRE(pretty_print(s) == "%? = const.i8 0x0");
}
