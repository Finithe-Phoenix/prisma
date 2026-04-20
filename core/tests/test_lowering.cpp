// core/tests/test_lowering.cpp — unit tests for the IR → ARM64 Lowerer.
//
// Two layers of assertion per test: (1) the Lowerer reports success, and
// (2) the disassembled output contains the expected mnemonics / operands.
// We don't pin exact bytes here because vixl picks encodings (e.g. mov
// alias) — the disassembler view is the stable one.

#include <catch2/catch_test_macros.hpp>
#include <string>

#include "prisma/emitter.hpp"
#include "prisma/ir.hpp"
#include "prisma/lowering.hpp"

using namespace prisma;

namespace {

std::string lower_to_disasm(std::span<const ir::Stmt> stmts, bool& ok) {
    backend::Emitter em;
    backend::Lowerer lw(em);
    const auto res = lw.lower(stmts);
    ok = res.success;
    if (!res.success) return {};
    em.finalize();
    return em.disassemble();
}

}  // namespace

TEST_CASE("Lowerer: Constant + StoreReg + Return → mov + mov + ret") {
    // %0 = const 42
    //      storereg rax, %0
    //      ret
    std::vector<ir::Stmt> stmts = {
        {0u, ir::Constant{42, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 0u, ir::OpSize::I64}},
        {std::nullopt, ir::Return{}},
    };

    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);

    // Expected ARM64 (per the fixed mapping rax → x10, first scratch = x0):
    //   mov x0, #0x2a     ; constant 42
    //   mov x10, x0       ; storereg rax
    //   ret
    REQUIRE(d.find("#0x2a")       != std::string::npos);
    REQUIRE(d.find("x0")          != std::string::npos);
    REQUIRE(d.find("x10")         != std::string::npos);
    REQUIRE(d.find("ret")         != std::string::npos);
}

TEST_CASE("Lowerer: LoadReg + BinOp + StoreReg → full ALU chain") {
    // %0 = loadreg rax
    // %1 = loadreg rbx
    // %2 = add %0, %1
    //      storereg rax, %2
    //      ret
    std::vector<ir::Stmt> stmts = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}},
        {std::nullopt, ir::Return{}},
    };

    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);

    // Expected (scratch allocation: %0→x0, %1→x1, %2→x2 ; rax→x10, rbx→x13
    // because x86 register encoding order is rax,rcx,rdx,rbx,...):
    //   mov x0, x10   ; loadreg rax
    //   mov x1, x13   ; loadreg rbx
    //   add x2, x0, x1
    //   mov x10, x2   ; storereg rax
    //   ret
    REQUIRE(d.find("add x2, x0, x1") != std::string::npos);
    REQUIRE(d.find("x10")            != std::string::npos);
    REQUIRE(d.find("x13")            != std::string::npos);
    REQUIRE(d.find("ret")            != std::string::npos);
}

TEST_CASE("Lowerer: each BinOpKind emits the right ARM64 mnemonic") {
    auto try_op = [](ir::BinOpKind k) {
        std::vector<ir::Stmt> s = {
            {0u, ir::Constant{1, ir::OpSize::I64}},
            {1u, ir::Constant{2, ir::OpSize::I64}},
            {2u, ir::BinOp{k, 0u, 1u, ir::OpSize::I64}},
        };
        bool ok;
        return std::pair{ok, lower_to_disasm(s, ok)};
    };

    {
        auto [ok, d] = try_op(ir::BinOpKind::Add);
        REQUIRE(ok); REQUIRE(d.find("add") != std::string::npos);
    }
    {
        auto [ok, d] = try_op(ir::BinOpKind::Sub);
        REQUIRE(ok); REQUIRE(d.find("sub") != std::string::npos);
    }
    {
        auto [ok, d] = try_op(ir::BinOpKind::And);
        REQUIRE(ok); REQUIRE(d.find("and") != std::string::npos);
    }
    {
        auto [ok, d] = try_op(ir::BinOpKind::Or);
        REQUIRE(ok); REQUIRE(d.find("orr") != std::string::npos);
    }
    {
        auto [ok, d] = try_op(ir::BinOpKind::Xor);
        REQUIRE(ok); REQUIRE(d.find("eor") != std::string::npos);
    }
    {
        auto [ok, d] = try_op(ir::BinOpKind::Shl);
        REQUIRE(ok); REQUIRE(d.find("lsl") != std::string::npos);
    }
    {
        auto [ok, d] = try_op(ir::BinOpKind::Shr);
        REQUIRE(ok); REQUIRE(d.find("lsr") != std::string::npos);
    }
    {
        auto [ok, d] = try_op(ir::BinOpKind::Sar);
        REQUIRE(ok); REQUIRE(d.find("asr") != std::string::npos);
    }
}

TEST_CASE("Lowerer: DanglingRef error on StoreReg that references unknown Ref") {
    // Missing the Constant that should bind %0.
    std::vector<ir::Stmt> stmts = {
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 0u, ir::OpSize::I64}},
    };
    backend::Emitter em;
    backend::Lowerer lw(em);
    auto r = lw.lower(stmts);
    REQUIRE_FALSE(r.success);
    REQUIRE(r.error == backend::LowerError::DanglingRef);
}

TEST_CASE("Lowerer: UnsupportedOp for ops not yet implemented") {
    // Jump is in the IR but the Lowerer does not emit it yet.
    std::vector<ir::Stmt> stmts = {
        {std::nullopt, ir::Jump{0u}},
    };
    backend::Emitter em;
    backend::Lowerer lw(em);
    auto r = lw.lower(stmts);
    REQUIRE_FALSE(r.success);
    REQUIRE(r.error == backend::LowerError::UnsupportedOp);
}

TEST_CASE("Lowerer: Compare → cmp + cset emits the right ARM64") {
    std::vector<ir::Stmt> stmts = {
        {0u, ir::Constant{10, ir::OpSize::I64}},
        {1u, ir::Constant{20, ir::OpSize::I64}},
        {2u, ir::Compare{ir::CondCode::Ult, 0u, 1u, ir::OpSize::I64}},
    };
    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);
    // Expected:
    //   mov x0, #10
    //   mov x1, #20
    //   cmp x0, x1
    //   cset x2, lo      (ARM mnemonic for unsigned-less-than)
    REQUIRE(d.find("cmp x0, x1") != std::string::npos);
    REQUIRE(d.find("cset")       != std::string::npos);
    // The condition suffix for Ult is "lo" on AArch64.
    REQUIRE(d.find("lo")         != std::string::npos);
}

TEST_CASE("Lowerer: Compare picks the correct ARM condition for each CondCode") {
    auto try_cc = [](ir::CondCode cc) {
        std::vector<ir::Stmt> s = {
            {0u, ir::Constant{1, ir::OpSize::I64}},
            {1u, ir::Constant{2, ir::OpSize::I64}},
            {2u, ir::Compare{cc, 0u, 1u, ir::OpSize::I64}},
        };
        bool ok;
        return std::pair{ok, lower_to_disasm(s, ok)};
    };

    // Spot-check the four most semantically different cases. The full
    // mapping lives in emitter.cpp::cset.
    auto [ok_eq,  d_eq ] = try_cc(ir::CondCode::Eq ); REQUIRE(ok_eq ); REQUIRE(d_eq .find(" eq") != std::string::npos);
    auto [ok_ne,  d_ne ] = try_cc(ir::CondCode::Ne ); REQUIRE(ok_ne ); REQUIRE(d_ne .find(" ne") != std::string::npos);
    auto [ok_slt, d_slt] = try_cc(ir::CondCode::Slt); REQUIRE(ok_slt); REQUIRE(d_slt.find(" lt") != std::string::npos);
    auto [ok_ugt, d_ugt] = try_cc(ir::CondCode::Ugt); REQUIRE(ok_ugt); REQUIRE(d_ugt.find(" hi") != std::string::npos);
}

TEST_CASE("Lowerer: OutOfScratchRegs when exceeding the 10-reg pool") {
    // 11 consecutive Constants → 11 scratch allocations → overflow.
    std::vector<ir::Stmt> stmts;
    for (unsigned i = 0; i < 11; ++i) {
        stmts.push_back({i, ir::Constant{i, ir::OpSize::I64}});
    }
    backend::Emitter em;
    backend::Lowerer lw(em);
    auto r = lw.lower(stmts);
    REQUIRE_FALSE(r.success);
    REQUIRE(r.error == backend::LowerError::OutOfScratchRegs);
}
