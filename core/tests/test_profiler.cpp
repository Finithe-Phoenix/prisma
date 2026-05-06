// core/tests/test_profiler.cpp — F1-IR-020.

#include <catch2/catch_test_macros.hpp>

#include "prisma/ir.hpp"
#include "prisma/profiler.hpp"

using namespace prisma;
using namespace prisma::ir;

TEST_CASE("OpCounter: tally increments for the matching variant") {
    OpCounter c;
    c.visit(Stmt{0u, Constant{42, OpSize::I64}});
    REQUIRE(c.count(OpCounter::Kind::Constant) == 1);
    REQUIRE(c.count(OpCounter::Kind::BinOp) == 0);
    REQUIRE(c.total() == 1);
}

TEST_CASE("OpCounter: visit a Stmt list rolls all counts up") {
    OpCounter c;
    std::vector<Stmt> stmts = {
        {0u, Constant{1, OpSize::I64}},
        {1u, Constant{2, OpSize::I64}},
        {2u, BinOp{BinOpKind::Add, 0u, 1u, OpSize::I64}},
        {std::nullopt, StoreReg{Gpr::Rax, 2u, OpSize::I64}},
        {std::nullopt, Return{}},
    };
    c.visit(stmts);
    REQUIRE(c.count(OpCounter::Kind::Constant) == 2);
    REQUIRE(c.count(OpCounter::Kind::BinOp)    == 1);
    REQUIRE(c.count(OpCounter::Kind::StoreReg) == 1);
    REQUIRE(c.count(OpCounter::Kind::Return)   == 1);
    REQUIRE(c.total() == 5);
}

TEST_CASE("OpCounter: visit a Function rolls every block in") {
    OpCounter c;
    Function fn;
    fn.entry = 0;
    fn.blocks.push_back(BasicBlock{0u, {
        {0u, Constant{1, OpSize::I64}}, {std::nullopt, Jump{1u}}}});
    fn.blocks.push_back(BasicBlock{1u, {
        {1u, Constant{2, OpSize::I64}}, {std::nullopt, Return{}}}});
    c.visit(fn);
    REQUIRE(c.count(OpCounter::Kind::Constant) == 2);
    REQUIRE(c.count(OpCounter::Kind::Jump) == 1);
    REQUIRE(c.count(OpCounter::Kind::Return) == 1);
    REQUIRE(c.total() == 4);
}

TEST_CASE("OpCounter: reset zeroes everything") {
    OpCounter c;
    c.visit(Stmt{std::nullopt, Return{}});
    REQUIRE(c.total() == 1);
    c.reset();
    REQUIRE(c.total() == 0);
    REQUIRE(c.count(OpCounter::Kind::Return) == 0);
}

TEST_CASE("OpCounter: snapshot captures the current counts") {
    OpCounter c;
    c.visit(Stmt{0u, Constant{1, OpSize::I64}});
    c.visit(Stmt{1u, Constant{2, OpSize::I64}});
    auto snap = c.snapshot();
    REQUIRE(snap[static_cast<std::size_t>(OpCounter::Kind::Constant)] == 2);
    // mutating after snapshot doesn't change the captured array
    c.visit(Stmt{std::nullopt, Return{}});
    REQUIRE(snap[static_cast<std::size_t>(OpCounter::Kind::Constant)] == 2);
}

TEST_CASE("OpCounter: Kind covers every Op variant") {
    // Every variant exercised at least once. If a variant is added
    // without updating profiler.cpp, this test catches it via missed
    // increments.
    OpCounter c;
    c.visit(Stmt{0u, Constant{0, OpSize::I64}});
    c.visit(Stmt{1u, LoadReg{Gpr::Rax, OpSize::I64}});
    c.visit(Stmt{std::nullopt, StoreReg{Gpr::Rbx, 1u, OpSize::I64}});
    c.visit(Stmt{2u, LoadSegBase{SegmentReg::Fs}});
    c.visit(Stmt{3u, BinOp{BinOpKind::Add, 0u, 1u, OpSize::I64}});
    c.visit(Stmt{4u, Compare{CondCode::Eq, 0u, 1u, OpSize::I64}});
    c.visit(Stmt{5u, Select{CondCode::Ne, 0u, 1u, OpSize::I64}});
    c.visit(Stmt{6u, LoadMem{1u, OpSize::I64}});
    c.visit(Stmt{std::nullopt, StoreMem{1u, 6u, OpSize::I64}});
    c.visit(Stmt{7u, LoadMemTSO{1u, OpSize::I64}});
    c.visit(Stmt{std::nullopt, StoreMemTSO{1u, 7u, OpSize::I64}});
    c.visit(Stmt{std::nullopt, Jump{0u}});
    c.visit(Stmt{std::nullopt, CondJump{4u, 0u, 1u}});
    c.visit(Stmt{std::nullopt, Return{}});
    c.visit(Stmt{std::nullopt, JumpReg{1u}});
    c.visit(Stmt{std::nullopt, CmpFlags{0u, 1u, OpSize::I64}});
    c.visit(Stmt{std::nullopt, JumpRel{0x100ULL}});
    c.visit(Stmt{std::nullopt,
        CondJumpRel{CondCode::Eq, 0x100ULL, 0x200ULL}});
    c.visit(Stmt{std::nullopt, CallRel{0x100ULL, 0x105ULL}});
    c.visit(Stmt{std::nullopt, CallReg{1u, 0x105ULL}});
    c.visit(Stmt{std::nullopt, RetAdjusted{0u}});
    c.visit(Stmt{std::nullopt, Cpuid{}});
    c.visit(Stmt{std::nullopt, Syscall{}});
    c.visit(Stmt{std::nullopt, Trap{TrapKind::Sigtrap}});
    c.visit(Stmt{8u,
        Extend{1u, OpSize::I8, OpSize::I64, /*signed=*/true}});
    c.visit(Stmt{9u, Truncate{8u, OpSize::I8}});
    c.visit(Stmt{std::nullopt, Fence{FenceKind::Mfence}});
    c.visit(Stmt{std::nullopt, GuestPc{0x4000ULL}});
    c.visit(Stmt{std::nullopt, InlineAsm{{0x0F, 0x05}}});
    c.visit(Stmt{10u, FpConstant{0x3FF0'0000'0000'0000ULL, FpSize::F64}});
    c.visit(Stmt{11u, FpBinOp{FpBinOpKind::Add, 10u, 10u, FpSize::F64}});
    c.visit(Stmt{12u, WriteFlags{BinOpKind::Sub, 0u, 0u, OpSize::I64}});
    c.visit(Stmt{13u, ReadFlag{12u, FlagBit::Zero}});
    c.visit(Stmt{std::nullopt, CondJumpFlags{12u, CondCode::Eq, 0u, 1u}});
    c.visit(Stmt{std::nullopt, RspAdjust{-8}});
    c.visit(Stmt{14u, VecConstant{0u, 0u}});
    c.visit(Stmt{15u, VecBinOp{VecBinOpKind::Add, 14u, 14u, VecLane::B16}});
    c.visit(Stmt{16u, LoadVecReg{0u}});
    c.visit(Stmt{std::nullopt, StoreVecReg{0u, 16u}});
    c.visit(Stmt{17u,
        VecFpBinOp{VecFpBinOpKind::Add, 14u, 14u, VecFpSize::S4}});
    c.visit(Stmt{18u,
        VecFpScalarBinOp{VecFpBinOpKind::Add, 14u, 14u, FpSize::F32}});
    c.visit(Stmt{19u, LoadVec{0u}});
    c.visit(Stmt{std::nullopt, StoreVec{0u, 19u}});
    c.visit(Stmt{20u, XmmFromGpr{0u, OpSize::I64}});
    c.visit(Stmt{21u, GprFromXmm{14u, OpSize::I32}});
    c.visit(Stmt{22u, VecCmp{VecCmpKind::Eq, 14u, 14u, VecLane::B16}});
    c.visit(Stmt{23u, VecShuffle32x4{14u, 0xE4}});
    c.visit(Stmt{24u, VecUnpack{false, 14u, 14u, VecLane::B16}});
    c.visit(Stmt{25u, VecShiftImm{VecShiftKind::ShiftL, 14u, 1u, VecLane::S4}});
    c.visit(Stmt{26u, VecShiftBytes{true, 14u, 4u}});
    c.visit(Stmt{27u, IntToFpScalar{0u, OpSize::I32, FpSize::F32}});
    c.visit(Stmt{28u, FpToIntScalar{14u, FpSize::F64, OpSize::I64}});
    c.visit(Stmt{29u, FpCvtScalar{14u, 14u, FpSize::F32, FpSize::F64}});
    c.visit(Stmt{30u, VecShuffle2Src{false, 14u, 14u, 0x1B}});
    c.visit(Stmt{31u, VecInsertLane{14u, 0u, 3, VecLane::H8}});
    c.visit(Stmt{32u, VecExtractLaneU{14u, 5, VecLane::H8}});
    c.visit(Stmt{33u, VecMaskMsb{14u}});
    c.visit(Stmt{34u, WriteFlagsFp{14u, 14u, FpSize::F64}});
    c.visit(Stmt{35u, VecShuffleH4{false, 14u, 0xE4}});
    c.visit(Stmt{36u, VecMaskFp{14u, false}});
    c.visit(Stmt{37u,
        VecFpCompare{14u, 14u, FpSize::F32, VecFpCmpPred::Eq, true}});
    c.visit(Stmt{38u, VecPshufb{14u, 14u}});
    c.visit(Stmt{39u, VecAbs{14u, VecLane::B16}});
    c.visit(Stmt{40u, VecAlignr{14u, 14u, 4u}});
    c.visit(Stmt{41u, VecExtend{14u, VecLane::B16, VecLane::H8, false}});
    c.visit(Stmt{42u, VecFpRound{14u, 14u, FpSize::F64, 0u, true}});
    c.visit(Stmt{43u, Popcnt{0u, OpSize::I64}});
    c.visit(Stmt{44u, Lzcnt{0u, OpSize::I64}});
    c.visit(Stmt{45u, Tzcnt{0u, OpSize::I32}});
    c.visit(Stmt{46u, VecBlend{14u, 14u, 14u, VecLane::B16}});
    c.visit(Stmt{47u, WriteFlagsPtest{14u, 14u}});

    REQUIRE(c.total() ==
            static_cast<std::uint64_t>(OpCounter::Kind::kCount));
}
