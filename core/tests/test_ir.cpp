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

    Stmt s_ret{std::nullopt, Return{}};
    REQUIRE(pretty_print(s_ret) == "ret");

    Stmt s_store_tso{std::nullopt, StoreMemTSO{/*addr=*/5u, /*value=*/6u, OpSize::I32}};
    REQUIRE(pretty_print(s_store_tso) == "store.tso.i32 [%5], %6");
}

TEST_CASE("kInvalidRef renders as %?") {
    Stmt s{kInvalidRef, Constant{0, OpSize::I8}};
    REQUIRE(pretty_print(s) == "%? = const.i8 0x0");
}

// ---------------------------------------------------------------------
// F1-IR-014 GuestPc pseudo-op
// ---------------------------------------------------------------------

TEST_CASE("InlineAsm carries opaque guest bytes and equates by content") {
    Stmt a{std::nullopt, InlineAsm{{0x0F, 0x05}}};   // SYSCALL bytes
    Stmt b{std::nullopt, InlineAsm{{0x0F, 0x05}}};
    Stmt c{std::nullopt, InlineAsm{{0x0F, 0x07}}};   // SYSRET bytes
    REQUIRE(a == b);
    REQUIRE_FALSE(a == c);
    REQUIRE(pretty_print(a) == "inline_asm 2B");
}

TEST_CASE("GuestPc has structural equality and prints with hex address") {
    Stmt a{std::nullopt, GuestPc{0xCAFE'BABE'1234'5678ull}};
    Stmt b{std::nullopt, GuestPc{0xCAFE'BABE'1234'5678ull}};
    Stmt c{std::nullopt, GuestPc{0xDEAD'BEEF'0000'0000ull}};
    REQUIRE(a == b);
    REQUIRE_FALSE(a == c);
    REQUIRE(pretty_print(a) == "guest_pc 0xcafebabe12345678");
}

// ---------------------------------------------------------------------
// F1-IR-019 memoised pretty-print
// ---------------------------------------------------------------------

TEST_CASE("pretty_print_memoised: identical Stmts share a string entry") {
    pretty_print_memoised_clear();
    Stmt a{0u, Constant{42, OpSize::I64}};
    Stmt b{0u, Constant{42, OpSize::I64}};
    const std::string sa = pretty_print_memoised(a);
    REQUIRE(pretty_print_memoised_size() == 1);
    const std::string sb = pretty_print_memoised(b);
    REQUIRE(pretty_print_memoised_size() == 1);  // dedup hit
    REQUIRE(sa == sb);
    REQUIRE(sa == "%0 = const.i64 0x2a");
}

TEST_CASE("pretty_print_memoised: distinct Stmts grow the cache") {
    pretty_print_memoised_clear();
    for (unsigned i = 0; i < 10; ++i) {
        Stmt s{i, Constant{i, OpSize::I64}};
        (void)pretty_print_memoised(s);
    }
    REQUIRE(pretty_print_memoised_size() == 10);
    pretty_print_memoised_clear();
    REQUIRE(pretty_print_memoised_size() == 0);
}

TEST_CASE("pretty_print_memoised: agrees byte-for-byte with the un-memoised path") {
    pretty_print_memoised_clear();
    Stmt s_extend{1u,
        Extend{/*value=*/0u, OpSize::I8, OpSize::I64, /*signed=*/true}};
    REQUIRE(pretty_print_memoised(s_extend) == pretty_print(s_extend));

    Stmt s_fence{std::nullopt, Fence{FenceKind::Mfence}};
    REQUIRE(pretty_print_memoised(s_fence) == pretty_print(s_fence));

    Stmt s_pc{std::nullopt, GuestPc{0x4000ULL}};
    REQUIRE(pretty_print_memoised(s_pc) == pretty_print(s_pc));
}
