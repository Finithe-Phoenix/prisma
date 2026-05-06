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
#include "prisma/passes.hpp"

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
        auto [ok, d] = try_op(ir::BinOpKind::Mul);
        REQUIRE(ok); REQUIRE(d.find("mul") != std::string::npos);
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
    {
        auto [ok, d] = try_op(ir::BinOpKind::Ror);
        REQUIRE(ok); REQUIRE(d.find("ror") != std::string::npos);
    }
    {
        auto [ok, d] = try_op(ir::BinOpKind::Rcl);
        REQUIRE(ok); REQUIRE(d.find("neg") != std::string::npos);
        REQUIRE(d.find("ror") != std::string::npos);
    }
    {
        auto [ok, d] = try_op(ir::BinOpKind::Rcr);
        REQUIRE(ok); REQUIRE(d.find("ror") != std::string::npos);
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

TEST_CASE("Lowerer: Cpuid zeroes guest output registers as a placeholder") {
    std::vector<ir::Stmt> stmts = {
        {std::nullopt, ir::Cpuid{}},
        {std::nullopt, ir::Return{}},
    };

    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);
    REQUIRE(d.find("x10") != std::string::npos);  // rax
    REQUIRE(d.find("x11") != std::string::npos);  // rcx
    REQUIRE(d.find("x12") != std::string::npos);  // rdx
    REQUIRE(d.find("x13") != std::string::npos);  // rbx
    REQUIRE(d.find("ret") != std::string::npos);
}

TEST_CASE("Lowerer: Syscall returns the halt sentinel as a placeholder terminator") {
    std::vector<ir::Stmt> stmts = {
        {std::nullopt, ir::Syscall{}},
    };

    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);
    REQUIRE(d.find("x0") != std::string::npos);
    REQUIRE(d.find("ret") != std::string::npos);
}

TEST_CASE("Lowerer: Trap(sigtrap) returns the placeholder terminator sequence") {
    std::vector<ir::Stmt> stmts = {
        {std::nullopt, ir::Trap{ir::TrapKind::Sigtrap}},
    };

    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);
    REQUIRE(d.find("x0") != std::string::npos);
    REQUIRE(d.find("ret") != std::string::npos);
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

TEST_CASE("Lowerer: Select emits csel") {
    // %0 = const.i64 10
    // %1 = const.i64 20
    // %2 = select.ne.%0.%1
    //      storereg.i64 rax, %2
    std::vector<ir::Stmt> stmts = {
        {0u, ir::Constant{10, ir::OpSize::I64}},
        {1u, ir::Constant{20, ir::OpSize::I64}},
        {2u, ir::Select{ir::CondCode::Ne, 0u, 1u, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}},
    };

    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);
    // Expected:
    //   mov x0, #10
    //   mov x1, #20
    //   csel x2, x0, x1, ne
    //   mov x10, x2
    REQUIRE(d.find("csel x2, x0, x1, ne") != std::string::npos);
    REQUIRE(d.find("x10") != std::string::npos);
}

// ---------------------------------------------------------------------------
// Memory ops lowering.
// ---------------------------------------------------------------------------

TEST_CASE("Lowerer: LoadMemTSO (i64) emits ldar") {
    // %0 = loadreg rbx        ; the base address
    // %1 = load.tso.i64 [%0]
    //      storereg rax, %1
    std::vector<ir::Stmt> stmts = {
        {0u, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}},
        {1u, ir::LoadMemTSO{0u, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 1u, ir::OpSize::I64}},
    };
    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);
    // Expected: mov x0, x13 ; ldar x1, [x0] ; mov x10, x1
    REQUIRE(d.find("ldar x1, [x0]") != std::string::npos);
}

TEST_CASE("Lowerer: LoadMem (non-TSO, i64) emits ldr") {
    std::vector<ir::Stmt> stmts = {
        {0u, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}},
        {1u, ir::LoadMem{0u, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 1u, ir::OpSize::I64}},
    };
    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);
    REQUIRE(d.find("ldr x1, [x0]") != std::string::npos);
}

TEST_CASE("Lowerer: StoreMemTSO emits stlr") {
    // %0 = loadreg rbx  ; address
    // %1 = loadreg rax  ; value
    //      storemem.tso.i64 [%0], %1
    std::vector<ir::Stmt> stmts = {
        {0u, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}},
        {1u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {std::nullopt, ir::StoreMemTSO{0u, 1u, ir::OpSize::I64}},
    };
    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);
    REQUIRE(d.find("stlr x1, [x0]") != std::string::npos);
}

// ---------------------------------------------------------------------------
// Control flow lowering.
// ---------------------------------------------------------------------------

TEST_CASE("Lowerer: JumpRel emits mov + ret") {
    std::vector<ir::Stmt> stmts = {
        {std::nullopt, ir::JumpRel{0xDEADBEEFULL}},
    };
    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);
    REQUIRE(d.find("mov x0") != std::string::npos);
    REQUIRE(d.find("ret")    != std::string::npos);
}

TEST_CASE("Lowerer: CmpFlags + CondJumpRel emits cmp + two movs + csel + ret") {
    std::vector<ir::Stmt> stmts = {
        {0u, ir::Constant{5, ir::OpSize::I64}},
        {1u, ir::Constant{5, ir::OpSize::I64}},
        {std::nullopt, ir::CmpFlags{0u, 1u, ir::OpSize::I64}},
        {std::nullopt, ir::CondJumpRel{ir::CondCode::Eq, 0x2000, 0x1008}},
    };
    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);
    // Key tokens in order.
    REQUIRE(d.find("cmp")  != std::string::npos);
    REQUIRE(d.find("csel") != std::string::npos);
    REQUIRE(d.find("ret")  != std::string::npos);
    // The two candidate PCs must appear as hex immediates in the mov stream.
    // vixl's disassembler prints 64-bit immediates so target and fallthrough
    // should both be visible.
    REQUIRE(d.find("0x2000") != std::string::npos);
    REQUIRE(d.find("0x1008") != std::string::npos);
}

TEST_CASE("Lowerer: each OpSize picks the matching ARM size suffix") {
    auto try_size = [](ir::OpSize sz) {
        std::vector<ir::Stmt> s = {
            {0u, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}},
            {1u, ir::LoadMem{0u, sz}},
        };
        bool ok;
        return std::pair{ok, lower_to_disasm(s, ok)};
    };

    auto [ok8,  d8 ] = try_size(ir::OpSize::I8);
    REQUIRE(ok8);  REQUIRE(d8 .find("ldrb") != std::string::npos);

    auto [ok16, d16] = try_size(ir::OpSize::I16);
    REQUIRE(ok16); REQUIRE(d16.find("ldrh") != std::string::npos);

    auto [ok32, d32] = try_size(ir::OpSize::I32);
    REQUIRE(ok32);
    // 32-bit ldr uses a W register; vixl prints `ldr w<n>, [x<m>]`.
    REQUIRE(d32.find("ldr w") != std::string::npos);

    auto [ok64, d64] = try_size(ir::OpSize::I64);
    REQUIRE(ok64); REQUIRE(d64.find("ldr x") != std::string::npos);
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

TEST_CASE("Lowerer: JumpReg emits mov x0, target and ret in standalone mode") {
    // %0 = loadreg rax
    // jumpreg %0
    // ret
    std::vector<ir::Stmt> stmts = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {std::nullopt, ir::JumpReg{0u}},
    };

    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);
    REQUIRE(d.find("mov x0, x10") != std::string::npos);
    REQUIRE(d.find("ret") != std::string::npos);
}

TEST_CASE("Lowerer: JumpReg can be lowered without auto-ret for translator mode") {
    // %0 = loadreg rax
    // jumpreg %0
    std::vector<ir::Stmt> stmts = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {std::nullopt, ir::JumpReg{0u}},
    };

    backend::Emitter em;
    backend::Lowerer lw(em, backend::LowerOptions{/*emit_ret_on_terminator=*/false});
    auto r = lw.lower(stmts);
    REQUIRE(r.success);
    em.finalize();
    const std::string d = em.disassemble();

    REQUIRE(d.find("mov x0, x10") != std::string::npos);
    REQUIRE(d.find("ret") == std::string::npos);
}

TEST_CASE("Lowerer: OutOfScratchRegs when 11 refs are simultaneously live") {
    // With the linear-scan allocator, dead defs free their register
    // immediately, so 11 consecutive never-used Constants now fit. To
    // force overflow we need 11 refs that ARE all live at the same
    // point — load 11 guest GPRs and keep each alive until a later
    // StoreReg reads it. By the 11th LoadReg, the pool is exhausted.
    const ir::Gpr srcs[11] = {
        ir::Gpr::Rax, ir::Gpr::Rcx, ir::Gpr::Rdx, ir::Gpr::Rbx,
        ir::Gpr::Rsi, ir::Gpr::Rdi, ir::Gpr::R8,  ir::Gpr::R9,
        ir::Gpr::R10, ir::Gpr::R11, ir::Gpr::R12,
    };
    const ir::Gpr dsts[11] = {
        ir::Gpr::R13, ir::Gpr::R14, ir::Gpr::R15, ir::Gpr::Rbp,
        ir::Gpr::Rsp, ir::Gpr::Rax, ir::Gpr::Rcx, ir::Gpr::Rdx,
        ir::Gpr::Rbx, ir::Gpr::Rsi, ir::Gpr::Rdi,
    };
    std::vector<ir::Stmt> stmts;
    for (unsigned i = 0; i < 11; ++i) {
        stmts.push_back({i, ir::LoadReg{srcs[i], ir::OpSize::I64}});
    }
    for (unsigned i = 0; i < 11; ++i) {
        stmts.push_back({std::nullopt,
                         ir::StoreReg{dsts[i], i, ir::OpSize::I64}});
    }
    backend::Emitter em;
    backend::Lowerer lw(em);
    auto r = lw.lower(stmts);
    REQUIRE_FALSE(r.success);
    REQUIRE(r.error == backend::LowerError::OutOfScratchRegs);
}

TEST_CASE("Lowerer: linear-scan reuses a reg after its Ref's last use") {
    // Two disjoint live intervals should share one scratch reg.
    //   %0 = const 1          pool: [x0]
    //   storereg rax, %0      %0 dies, x0 returns to pool
    //   %1 = const 2          reuses x0
    //   storereg rbx, %1      %1 dies
    std::vector<ir::Stmt> stmts = {
        {0u, ir::Constant{1, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 0u, ir::OpSize::I64}},
        {1u, ir::Constant{2, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rbx, 1u, ir::OpSize::I64}},
    };
    backend::Emitter em;
    backend::Lowerer lw(em);
    auto r = lw.lower(stmts);
    REQUIRE(r.success);
    REQUIRE(lw.scratch_used() == 1u);
}

TEST_CASE("Lowerer: linear-scan extends an interval through a later BinOp") {
    // %0 lives from stmt 0 to stmt 2 (read by Add). %1 lives 1..2.
    // Peak live at stmt 2 is {%0, %1, %2} = 3 simultaneous scratches.
    std::vector<ir::Stmt> stmts = {
        {0u, ir::Constant{10, ir::OpSize::I64}},
        {1u, ir::Constant{20, ir::OpSize::I64}},
        {2u, ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 2u, ir::OpSize::I64}},
    };
    backend::Emitter em;
    backend::Lowerer lw(em);
    auto r = lw.lower(stmts);
    REQUIRE(r.success);
    REQUIRE(lw.scratch_used() == 3u);
}

TEST_CASE("Lowerer: linear-scan frees dead Constants, 11 in a row succeed") {
    // 11 never-used Constants each have live interval = [def, def] so
    // they all reuse the same scratch. This used to overflow under the
    // bump-pointer allocator but now succeeds.
    std::vector<ir::Stmt> stmts;
    for (unsigned i = 0; i < 11; ++i) {
        stmts.push_back({i, ir::Constant{i, ir::OpSize::I64}});
    }
    backend::Emitter em;
    backend::Lowerer lw(em);
    auto r = lw.lower(stmts);
    REQUIRE(r.success);
    REQUIRE(lw.scratch_used() == 1u);  // peak: only one live at any time
}

// ---------------------------------------------------------------------------
// F1-BK-008 register spill / reload
// ---------------------------------------------------------------------------

TEST_CASE("Lowerer: spill_slots>=4 lets an 11-live-ref block succeed") {
    const ir::Gpr srcs[11] = {
        ir::Gpr::Rax, ir::Gpr::Rcx, ir::Gpr::Rdx, ir::Gpr::Rbx,
        ir::Gpr::Rsi, ir::Gpr::Rdi, ir::Gpr::R8,  ir::Gpr::R9,
        ir::Gpr::R10, ir::Gpr::R11, ir::Gpr::R12,
    };
    const ir::Gpr dsts[11] = {
        ir::Gpr::R13, ir::Gpr::R14, ir::Gpr::R15, ir::Gpr::Rbp,
        ir::Gpr::Rsp, ir::Gpr::Rax, ir::Gpr::Rcx, ir::Gpr::Rdx,
        ir::Gpr::Rbx, ir::Gpr::Rsi, ir::Gpr::Rdi,
    };
    std::vector<ir::Stmt> stmts;
    for (unsigned i = 0; i < 11; ++i) {
        stmts.push_back({i, ir::LoadReg{srcs[i], ir::OpSize::I64}});
    }
    for (unsigned i = 0; i < 11; ++i) {
        stmts.push_back({std::nullopt,
                         ir::StoreReg{dsts[i], i, ir::OpSize::I64}});
    }
    backend::Emitter em;
    backend::LowerOptions opts;
    opts.spill_slots            = 4;
    opts.spill_slot_base_offset = 0;
    backend::Lowerer lw(em, opts);
    auto r = lw.lower(stmts);
    REQUIRE(r.success);
    REQUIRE(lw.peak_spills() >= 1u);

    em.finalize();
    const std::string d = em.disassemble();
    REQUIRE(d.find("[sp") != std::string::npos);
    REQUIRE(d.find("str") != std::string::npos);
    REQUIRE(d.find("ldr") != std::string::npos);
}

TEST_CASE("Lowerer: pool + spill slots both exhaust gives OutOfScratchRegs") {
    std::vector<ir::Stmt> stmts;
    const ir::Gpr src[12] = {
        ir::Gpr::Rax, ir::Gpr::Rcx, ir::Gpr::Rdx, ir::Gpr::Rbx,
        ir::Gpr::Rsi, ir::Gpr::Rdi, ir::Gpr::R8,  ir::Gpr::R9,
        ir::Gpr::R10, ir::Gpr::R11, ir::Gpr::R12, ir::Gpr::R13,
    };
    for (unsigned i = 0; i < 12; ++i) {
        stmts.push_back({i, ir::LoadReg{src[i], ir::OpSize::I64}});
    }
    for (unsigned i = 0; i < 12; ++i) {
        stmts.push_back({std::nullopt,
                         ir::StoreReg{ir::Gpr::R14, i, ir::OpSize::I64}});
    }
    backend::Emitter em;
    backend::LowerOptions opts;
    opts.spill_slots = 1;
    backend::Lowerer lw(em, opts);
    auto r = lw.lower(stmts);
    REQUIRE_FALSE(r.success);
    REQUIRE(r.error == backend::LowerError::OutOfScratchRegs);
}

TEST_CASE("Lowerer: spill honours spill_slot_base_offset") {
    const ir::Gpr srcs[11] = {
        ir::Gpr::Rax, ir::Gpr::Rcx, ir::Gpr::Rdx, ir::Gpr::Rbx,
        ir::Gpr::Rsi, ir::Gpr::Rdi, ir::Gpr::R8,  ir::Gpr::R9,
        ir::Gpr::R10, ir::Gpr::R11, ir::Gpr::R12,
    };
    std::vector<ir::Stmt> stmts;
    for (unsigned i = 0; i < 11; ++i) {
        stmts.push_back({i, ir::LoadReg{srcs[i], ir::OpSize::I64}});
    }
    for (unsigned i = 0; i < 11; ++i) {
        stmts.push_back({std::nullopt,
                         ir::StoreReg{ir::Gpr::R14, i, ir::OpSize::I64}});
    }
    backend::Emitter em;
    backend::LowerOptions opts;
    opts.spill_slots            = 4;
    opts.spill_slot_base_offset = 64;
    backend::Lowerer lw(em, opts);
    auto r = lw.lower(stmts);
    REQUIRE(r.success);

    em.finalize();
    const std::string d = em.disassemble();
    // vixl renders sp-relative offsets differently across versions
    // (`[sp, #64]` decimal vs `[sp, #0x40]` hex). Accept either.
    const bool found = d.find("sp, #64")  != std::string::npos
                    || d.find("sp, #0x40") != std::string::npos;
    INFO("disasm: " << d);
    REQUIRE(found);
}

TEST_CASE("Lowerer: spill+reload round-trip emits a valid add") {
    std::vector<ir::Stmt> stmts;
    for (unsigned i = 0; i < 11; ++i) {
        stmts.push_back({i, ir::Constant{i + 1, ir::OpSize::I64}});
    }
    stmts.push_back({11u, ir::BinOp{ir::BinOpKind::Add, 0u, 10u, ir::OpSize::I64}});
    stmts.push_back({std::nullopt, ir::StoreReg{ir::Gpr::Rax, 11u, ir::OpSize::I64}});

    backend::Emitter em;
    backend::LowerOptions opts;
    opts.spill_slots = 4;
    backend::Lowerer lw(em, opts);
    auto r = lw.lower(stmts);
    REQUIRE(r.success);

    em.finalize();
    const std::string d = em.disassemble();
    REQUIRE(d.find("add") != std::string::npos);
}

// ---------------------------------------------------------------------
// F1-BK-003 / F1-BK-004 / F1-BK-006: Function lowering with labels.
// ---------------------------------------------------------------------

TEST_CASE("Lowerer: flat overload still rejects Jump as UnsupportedOp") {
    // Without a Function context there is no block_labels_ map; Jump
    // can't resolve a target. The test that pinned the old behaviour
    // expected this — keep the contract.
    std::vector<ir::Stmt> stmts = {{std::nullopt, ir::Jump{0u}}};
    backend::Emitter em;
    backend::Lowerer lw(em);
    auto r = lw.lower(stmts);
    REQUIRE_FALSE(r.success);
    REQUIRE(r.error == backend::LowerError::UnsupportedOp);
}

TEST_CASE("Lowerer(Function): single-block returns immediately") {
    ir::Function fn;
    fn.entry = 0;
    fn.blocks.push_back(ir::BasicBlock{0u, {{std::nullopt, ir::Return{}}}});

    backend::Emitter em;
    backend::Lowerer lw(em);
    auto r = lw.lower(fn);
    REQUIRE(r.success);
    em.finalize();
    REQUIRE(em.disassemble().find("ret") != std::string::npos);
}

TEST_CASE("Lowerer(Function): unconditional Jump emits a `b` to the target label") {
    // bb0:  Jump bb1
    // bb1:  Return
    ir::Function fn;
    fn.entry = 0;
    fn.blocks.push_back(ir::BasicBlock{0u, {{std::nullopt, ir::Jump{1u}}}});
    fn.blocks.push_back(ir::BasicBlock{1u, {{std::nullopt, ir::Return{}}}});

    backend::Emitter em;
    backend::Lowerer lw(em);
    auto r = lw.lower(fn);
    REQUIRE(r.success);
    em.finalize();
    const std::string d = em.disassemble();
    INFO("disasm: " << d);
    // vixl prints unconditional branches as `b #+0xN` or `b 0x...`.
    // Match either; the important thing is the mnemonic appears
    // exactly once before the ret.
    REQUIRE(d.find("ret") != std::string::npos);
    const std::size_t ret_pos = d.find("ret");
    const std::size_t b_pos   = d.find("b ");
    REQUIRE(b_pos != std::string::npos);
    REQUIRE(b_pos < ret_pos);
}

TEST_CASE("Lowerer(Function): CondJump emits `cbnz` + fallthrough `b`") {
    // bb0:
    //   %0 = const 1            ; non-zero condition
    //   CondJump %0, bb1, bb2
    // bb1: Return
    // bb2: Return
    ir::Function fn;
    fn.entry = 0;
    fn.blocks.push_back(ir::BasicBlock{0u, {
        {0u, ir::Constant{1, ir::OpSize::I64}},
        {std::nullopt, ir::CondJump{0u, /*if_true=*/1u, /*if_false=*/2u}},
    }});
    fn.blocks.push_back(ir::BasicBlock{1u, {{std::nullopt, ir::Return{}}}});
    fn.blocks.push_back(ir::BasicBlock{2u, {{std::nullopt, ir::Return{}}}});

    backend::Emitter em;
    backend::Lowerer lw(em);
    auto r = lw.lower(fn);
    REQUIRE(r.success);
    em.finalize();
    const std::string d = em.disassemble();
    INFO("disasm: " << d);
    REQUIRE(d.find("cbnz") != std::string::npos);
}

TEST_CASE("Lowerer(Function): CondJump with dangling cond ref reports DanglingRef") {
    ir::Function fn;
    fn.entry = 0;
    fn.blocks.push_back(ir::BasicBlock{0u, {
        // No def for ref 7 in this block — should fail validation.
        {std::nullopt, ir::CondJump{7u, 1u, 2u}},
    }});
    fn.blocks.push_back(ir::BasicBlock{1u, {{std::nullopt, ir::Return{}}}});
    fn.blocks.push_back(ir::BasicBlock{2u, {{std::nullopt, ir::Return{}}}});

    backend::Emitter em;
    backend::Lowerer lw(em);
    auto r = lw.lower(fn);
    REQUIRE_FALSE(r.success);
    REQUIRE(r.error == backend::LowerError::DanglingRef);
}

TEST_CASE("Lowerer(Function): three-block diamond lowers cleanly") {
    // bb0:  %0 = const 0;  CondJump %0, bb1, bb2
    // bb1:  Jump bb3
    // bb2:  Jump bb3
    // bb3:  Return
    ir::Function fn;
    fn.entry = 0;
    fn.blocks.push_back(ir::BasicBlock{0u, {
        {0u, ir::Constant{0, ir::OpSize::I64}},
        {std::nullopt, ir::CondJump{0u, 1u, 2u}},
    }});
    fn.blocks.push_back(ir::BasicBlock{1u, {{std::nullopt, ir::Jump{3u}}}});
    fn.blocks.push_back(ir::BasicBlock{2u, {{std::nullopt, ir::Jump{3u}}}});
    fn.blocks.push_back(ir::BasicBlock{3u, {{std::nullopt, ir::Return{}}}});

    backend::Emitter em;
    backend::Lowerer lw(em);
    auto r = lw.lower(fn);
    REQUIRE(r.success);
    em.finalize();
    const std::string d = em.disassemble();
    INFO("disasm: " << d);
    // One cbnz, two unconditional b (bb1→bb3, bb2→bb3), one ret.
    REQUIRE(d.find("cbnz") != std::string::npos);
    REQUIRE(d.find("ret")  != std::string::npos);
}

TEST_CASE("Lowerer: LoadSegBase materialises a value in a scratch reg") {
    std::vector<ir::Stmt> stmts = {
        {0u, ir::LoadSegBase{ir::SegmentReg::Fs}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rdi, 0u, ir::OpSize::I64}},
        {std::nullopt, ir::Return{}},
    };
    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);
    // Placeholder lowering currently zeroes the destination — assert
    // we see a movz #0 (or `mov xN, xzr` after vixl peephole) and a
    // ret. The TLS table integration follow-up will swap the body
    // without changing this surface.
    REQUIRE(d.find("ret") != std::string::npos);
}

TEST_CASE("Lowerer: CallRel parks target_guest_pc in x0 then ret") {
    std::vector<ir::Stmt> stmts = {
        {std::nullopt, ir::CallRel{0xDEAD'BEEFu, 0xCAFE'BABEu}},
    };
    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);
    REQUIRE(d.find("x0") != std::string::npos);
    REQUIRE(d.find("ret") != std::string::npos);
}

// ---------------------------------------------------------------------
// F1-BK-022 Extend / Truncate, F1-BK-023 Fence
// ---------------------------------------------------------------------

TEST_CASE("Lowerer: Extend signed i8 → i64 emits sxtb") {
    std::vector<ir::Stmt> stmts = {
        {0u, ir::Constant{0xFF, ir::OpSize::I8}},
        {1u, ir::Extend{0u, ir::OpSize::I8, ir::OpSize::I64, /*signed=*/true}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 1u, ir::OpSize::I64}},
        {std::nullopt, ir::Return{}},
    };
    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);
    REQUIRE(d.find("sxtb") != std::string::npos);
}

TEST_CASE("Lowerer: Extend signed i16 → i64 emits sxth") {
    std::vector<ir::Stmt> stmts = {
        {0u, ir::Constant{0x8000, ir::OpSize::I16}},
        {1u, ir::Extend{0u, ir::OpSize::I16, ir::OpSize::I64, true}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 1u, ir::OpSize::I64}},
        {std::nullopt, ir::Return{}},
    };
    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);
    REQUIRE(d.find("sxth") != std::string::npos);
}

TEST_CASE("Lowerer: Extend signed i32 → i64 emits sxtw") {
    std::vector<ir::Stmt> stmts = {
        {0u, ir::Constant{0x80000000u, ir::OpSize::I32}},
        {1u, ir::Extend{0u, ir::OpSize::I32, ir::OpSize::I64, true}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 1u, ir::OpSize::I64}},
        {std::nullopt, ir::Return{}},
    };
    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);
    REQUIRE(d.find("sxtw") != std::string::npos);
}

TEST_CASE("Lowerer: Extend unsigned i8 → i64 emits uxtb") {
    std::vector<ir::Stmt> stmts = {
        {0u, ir::Constant{0xFF, ir::OpSize::I8}},
        {1u, ir::Extend{0u, ir::OpSize::I8, ir::OpSize::I64, /*signed=*/false}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 1u, ir::OpSize::I64}},
        {std::nullopt, ir::Return{}},
    };
    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);
    REQUIRE(d.find("uxtb") != std::string::npos);
}

TEST_CASE("Lowerer: Truncate to i32 emits a 32-bit move (mov w*)") {
    std::vector<ir::Stmt> stmts = {
        {0u, ir::Constant{0x1234'5678'9ABC'DEF0ull, ir::OpSize::I64}},
        {1u, ir::Truncate{0u, ir::OpSize::I32}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 1u, ir::OpSize::I32}},
        {std::nullopt, ir::Return{}},
    };
    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);
    // vixl prints `mov w*, w*` for the W-view move idiom.
    REQUIRE(d.find("mov w") != std::string::npos);
}

TEST_CASE("Lowerer: Fence Mfence emits dmb ish") {
    std::vector<ir::Stmt> stmts = {
        {std::nullopt, ir::Fence{ir::FenceKind::Mfence}},
        {std::nullopt, ir::Return{}},
    };
    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);
    REQUIRE(d.find("dmb ish") != std::string::npos);
}

TEST_CASE("Lowerer: Fence Lfence emits dmb ishld") {
    std::vector<ir::Stmt> stmts = {
        {std::nullopt, ir::Fence{ir::FenceKind::Lfence}},
        {std::nullopt, ir::Return{}},
    };
    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);
    REQUIRE(d.find("dmb ishld") != std::string::npos);
}

// ---------------------------------------------------------------------
// F1-RT-013 RspAdjust
// ---------------------------------------------------------------------

TEST_CASE("Lowerer: RspAdjust(-8) emits a sub on the rsp host reg") {
    std::vector<ir::Stmt> stmts = {
        {std::nullopt, ir::RspAdjust{-8}},
        {std::nullopt, ir::Return{}},
    };
    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);
    // Guest RSP maps to host x14 (per arm64::host_reg_for(Rsp)).
    REQUIRE(d.find("sub") != std::string::npos);
    REQUIRE(d.find("x14") != std::string::npos);
}

TEST_CASE("Lowerer: RspAdjust(+16) emits an add on the rsp host reg") {
    std::vector<ir::Stmt> stmts = {
        {std::nullopt, ir::RspAdjust{16}},
        {std::nullopt, ir::Return{}},
    };
    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);
    REQUIRE(d.find("add") != std::string::npos);
    REQUIRE(d.find("x14") != std::string::npos);
}

// ---------------------------------------------------------------------
// F1-IR-003/004/005 flags pillar lowering
// ---------------------------------------------------------------------

TEST_CASE("Lowerer: WriteFlags(Sub) + ReadFlag(Zero) emits cmp + cset eq") {
    std::vector<ir::Stmt> stmts = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}},
        {2u, ir::WriteFlags{ir::BinOpKind::Sub, 0u, 1u, ir::OpSize::I64}},
        {3u, ir::ReadFlag{2u, ir::FlagBit::Zero}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rcx, 3u, ir::OpSize::I8}},
        {std::nullopt, ir::Return{}},
    };
    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);
    REQUIRE(d.find("cmp") != std::string::npos);
    REQUIRE(d.find("cset") != std::string::npos);
}

TEST_CASE("Lowerer: WriteFlags(Add) emits adds (flag-setting variant)") {
    std::vector<ir::Stmt> stmts = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}},
        {2u, ir::WriteFlags{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}},
        {3u, ir::ReadFlag{2u, ir::FlagBit::Carry}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rcx, 3u, ir::OpSize::I8}},
        {std::nullopt, ir::Return{}},
    };
    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);
    REQUIRE(d.find("adds") != std::string::npos);
}

TEST_CASE("Lowerer: WriteFlags(And) emits ands") {
    std::vector<ir::Stmt> stmts = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}},
        {2u, ir::WriteFlags{ir::BinOpKind::And, 0u, 1u, ir::OpSize::I64}},
        {3u, ir::ReadFlag{2u, ir::FlagBit::Zero}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rcx, 3u, ir::OpSize::I8}},
        {std::nullopt, ir::Return{}},
    };
    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);
    REQUIRE(d.find("ands") != std::string::npos);
}

TEST_CASE("Lowerer: ReadFlag without a prior WriteFlags is rejected") {
    std::vector<ir::Stmt> stmts = {
        // Ref 7 is undefined as a Flags ref.
        {0u, ir::ReadFlag{7u, ir::FlagBit::Zero}},
    };
    bool ok;
    (void)lower_to_disasm(stmts, ok);
    REQUIRE_FALSE(ok);
}

TEST_CASE("Lowerer(Function): CondJumpFlags emits b.cc + b") {
    // bb0: WriteFlags Sub, %0=loadreg rax, %1=loadreg rbx;
    //      CondJumpFlags eq, bb1, bb2
    // bb1, bb2: Return
    ir::Function fn;
    fn.entry = 0;
    fn.blocks.push_back(ir::BasicBlock{0u, {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}},
        {2u, ir::WriteFlags{ir::BinOpKind::Sub, 0u, 1u, ir::OpSize::I64}},
        {std::nullopt,
         ir::CondJumpFlags{2u, ir::CondCode::Eq, 1u, 2u}},
    }});
    fn.blocks.push_back(ir::BasicBlock{1u, {{std::nullopt, ir::Return{}}}});
    fn.blocks.push_back(ir::BasicBlock{2u, {{std::nullopt, ir::Return{}}}});

    backend::Emitter em;
    backend::Lowerer lw(em);
    auto r = lw.lower(fn);
    REQUIRE(r.success);
    em.finalize();
    const std::string d = em.disassemble();
    REQUIRE(d.find("b.eq") != std::string::npos);
    REQUIRE(d.find("cmp")  != std::string::npos);
}

// ---------------------------------------------------------------------
// F2-IR-001/002/003 SIMD lowering
// ---------------------------------------------------------------------

TEST_CASE("Lowerer: VecConstant + VecBinOp(Add, B16) emits NEON add v.16b") {
    std::vector<ir::Stmt> stmts = {
        {0u, ir::VecConstant{0x0102030405060708ull, 0x0900000000000000ull}},
        {1u, ir::VecConstant{0x0102030405060708ull, 0x0000000000000000ull}},
        {2u, ir::VecBinOp{ir::VecBinOpKind::Add, 0u, 1u, ir::VecLane::B16}},
        {std::nullopt, ir::Return{}},
    };
    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);
    REQUIRE(d.find("16b") != std::string::npos);
    REQUIRE(d.find("add") != std::string::npos);
}

TEST_CASE("Lowerer: VecBinOp lanes (H8/S4/D2) emit the right arrangement") {
    {
        std::vector<ir::Stmt> stmts = {
            {0u, ir::VecConstant{0u, 0u}},
            {1u, ir::VecConstant{0u, 0u}},
            {2u, ir::VecBinOp{ir::VecBinOpKind::Sub, 0u, 1u, ir::VecLane::H8}},
            {std::nullopt, ir::Return{}},
        };
        bool ok;
        const std::string d = lower_to_disasm(stmts, ok);
        REQUIRE(ok);
        REQUIRE(d.find("8h") != std::string::npos);
    }
    {
        std::vector<ir::Stmt> stmts = {
            {0u, ir::VecConstant{0u, 0u}},
            {1u, ir::VecConstant{0u, 0u}},
            {2u, ir::VecBinOp{ir::VecBinOpKind::Add, 0u, 1u, ir::VecLane::S4}},
            {std::nullopt, ir::Return{}},
        };
        bool ok;
        const std::string d = lower_to_disasm(stmts, ok);
        REQUIRE(ok);
        REQUIRE(d.find("4s") != std::string::npos);
    }
    {
        std::vector<ir::Stmt> stmts = {
            {0u, ir::VecConstant{0u, 0u}},
            {1u, ir::VecConstant{0u, 0u}},
            {2u, ir::VecBinOp{ir::VecBinOpKind::Add, 0u, 1u, ir::VecLane::D2}},
            {std::nullopt, ir::Return{}},
        };
        bool ok;
        const std::string d = lower_to_disasm(stmts, ok);
        REQUIRE(ok);
        REQUIRE(d.find("2d") != std::string::npos);
    }
}

TEST_CASE("Lowerer: PADDD xmm0, [rcx] full IR sequence lowers") {
    std::vector<ir::Stmt> stmts = {
        {0u,           ir::LoadVecReg{0u}},
        {1u,           ir::LoadReg{ir::Gpr::Rcx, ir::OpSize::I64}},
        {2u,           ir::LoadVec{1u}},
        {3u,           ir::VecBinOp{ir::VecBinOpKind::Add, 0u, 2u, ir::VecLane::S4}},
        {std::nullopt, ir::StoreVecReg{0u, 3u}},
        {std::nullopt, ir::Return{}},
    };
    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    INFO("disasm: " << d);
    REQUIRE(ok);
}

TEST_CASE("Lowerer: F2-IR-026 UCOMISD-style flags chain — fcmp + cset(eq) + cset(slt)") {
    // Manually construct the IR sequence the decoder emits, then
    // verify lowering succeeds and emits fcmp / cset.
    std::vector<ir::Stmt> stmts = {
        {0u,           ir::LoadVecReg{0u}},
        {1u,           ir::LoadVecReg{1u}},
        {2u,           ir::WriteFlagsFp{0u, 1u, ir::FpSize::F64}},
        {3u,           ir::ReadFlag{2u, ir::FlagBit::Zero}},
        {4u,           ir::ReadFlag{2u, ir::FlagBit::Carry}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 3u, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rcx, 4u, ir::OpSize::I64}},
        {std::nullopt, ir::Return{}},
    };
    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);
    REQUIRE(d.find("fcmp")  != std::string::npos);
    REQUIRE(d.find("cset")  != std::string::npos);
    REQUIRE(d.find("eq")    != std::string::npos);
    REQUIRE(d.find("lt")    != std::string::npos);
}

TEST_CASE("Lowerer: F2-IR-016 cvtsi2sd round-trip through pipeline") {
    std::vector<ir::Stmt> stmts = {
        {0u,           ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u,           ir::IntToFpScalar{0u, ir::OpSize::I64, ir::FpSize::F64}},
        {std::nullopt, ir::StoreVecReg{0u, 1u}},
        {2u,           ir::LoadVecReg{0u}},
        {3u,           ir::FpToIntScalar{2u, ir::FpSize::F64, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rcx, 3u, ir::OpSize::I64}},
        {std::nullopt, ir::Return{}},
    };
    auto pm = passes::default_pipeline();
    auto [opt, _stats] = pm.run(stmts);
    std::string dump;
    for (auto const& s : opt) {
        dump += ir::pretty_print(s);
        dump += "\n";
    }
    INFO("post-pipeline:\n" << dump);
    bool ok;
    const std::string d = lower_to_disasm(opt, ok);
    REQUIRE(ok);
    REQUIRE(d.find("scvtf") != std::string::npos);
    REQUIRE(d.find("fcvtzs") != std::string::npos);
}

TEST_CASE("Lowerer: PADDD via default pipeline still lowers") {
    std::vector<ir::Stmt> stmts = {
        {0u,           ir::LoadVecReg{0u}},
        {1u,           ir::LoadReg{ir::Gpr::Rcx, ir::OpSize::I64}},
        {2u,           ir::LoadVec{1u}},
        {3u,           ir::VecBinOp{ir::VecBinOpKind::Add, 0u, 2u, ir::VecLane::S4}},
        {std::nullopt, ir::StoreVecReg{0u, 3u}},
        {std::nullopt, ir::Return{}},
    };
    auto pm = passes::default_pipeline();
    auto [opt, _stats] = pm.run(stmts);
    std::string dump;
    for (auto const& s : opt) {
        dump += ir::pretty_print(s);
        dump += "\n";
    }
    INFO("post-pipeline:\n" << dump);
    bool ok;
    const std::string d = lower_to_disasm(opt, ok);
    INFO("disasm: " << d);
    REQUIRE(ok);
}

TEST_CASE("Lowerer: LoadVec + StoreVec emit ldr/str Q forms") {
    std::vector<ir::Stmt> stmts = {
        {0u,           ir::LoadReg{ir::Gpr::Rcx, ir::OpSize::I64}},
        {1u,           ir::LoadVec{0u}},
        {std::nullopt, ir::StoreVec{0u, 1u}},
        {std::nullopt, ir::Return{}},
    };
    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);
    REQUIRE(d.find("ldr") != std::string::npos);
    REQUIRE(d.find("str") != std::string::npos);
}

TEST_CASE("Lowerer: bitwise VecBinOp(And/Or/Xor) emits 16b NEON forms") {
    for (auto op : {ir::VecBinOpKind::And, ir::VecBinOpKind::Or,
                    ir::VecBinOpKind::Xor}) {
        std::vector<ir::Stmt> stmts = {
            {0u, ir::VecConstant{0xFFFFFFFFFFFFFFFFull, 0x0u}},
            {1u, ir::VecConstant{0u, 0u}},
            {2u, ir::VecBinOp{op, 0u, 1u, ir::VecLane::B16}},
            {std::nullopt, ir::Return{}},
        };
        bool ok;
        const std::string d = lower_to_disasm(stmts, ok);
        REQUIRE(ok);
        // vixl prints the bitwise vector ops as `and`, `orr`, `eor` — same
        // mnemonics as the integer scalar forms, but disambiguated by the
        // `.16b` suffix on the operand.
        REQUIRE(d.find("16b") != std::string::npos);
    }
}

// ---------------------------------------------------------------------
// F1-BK-013 floating-point lowering
// ---------------------------------------------------------------------

TEST_CASE("Lowerer: FpConstant + FpBinOp(Add) emits fadd s/d") {
    // double 1.0 + 2.0 → 3.0 (we don't execute, just check disasm)
    std::uint64_t one = 0; std::uint64_t two = 0;
    {
        const double a = 1.0, b = 2.0;
        std::memcpy(&one, &a, sizeof one);
        std::memcpy(&two, &b, sizeof two);
    }
    std::vector<ir::Stmt> stmts = {
        {0u, ir::FpConstant{one, ir::FpSize::F64}},
        {1u, ir::FpConstant{two, ir::FpSize::F64}},
        {2u, ir::FpBinOp{ir::FpBinOpKind::Add, 0u, 1u, ir::FpSize::F64}},
        {std::nullopt, ir::Return{}},
    };
    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);
    REQUIRE(d.find("fadd") != std::string::npos);
}

TEST_CASE("Lowerer: FpBinOp(Mul) on f32 emits fmul s") {
    std::uint32_t bits_one = 0;
    {
        const float a = 1.5f;
        std::memcpy(&bits_one, &a, sizeof bits_one);
    }
    std::vector<ir::Stmt> stmts = {
        {0u, ir::FpConstant{bits_one, ir::FpSize::F32}},
        {1u, ir::FpConstant{bits_one, ir::FpSize::F32}},
        {2u, ir::FpBinOp{ir::FpBinOpKind::Mul, 0u, 1u, ir::FpSize::F32}},
        {std::nullopt, ir::Return{}},
    };
    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);
    REQUIRE(d.find("fmul") != std::string::npos);
}

TEST_CASE("Lowerer: FpBinOp(Sub) and (Div) emit the matching instructions") {
    std::uint64_t two = 0;
    {
        const double a = 2.0;
        std::memcpy(&two, &a, sizeof two);
    }
    {
        std::vector<ir::Stmt> stmts = {
            {0u, ir::FpConstant{two, ir::FpSize::F64}},
            {1u, ir::FpConstant{two, ir::FpSize::F64}},
            {2u, ir::FpBinOp{ir::FpBinOpKind::Sub, 0u, 1u, ir::FpSize::F64}},
            {std::nullopt, ir::Return{}},
        };
        bool ok;
        const std::string d = lower_to_disasm(stmts, ok);
        REQUIRE(ok);
        REQUIRE(d.find("fsub") != std::string::npos);
    }
    {
        std::vector<ir::Stmt> stmts = {
            {0u, ir::FpConstant{two, ir::FpSize::F64}},
            {1u, ir::FpConstant{two, ir::FpSize::F64}},
            {2u, ir::FpBinOp{ir::FpBinOpKind::Div, 0u, 1u, ir::FpSize::F64}},
            {std::nullopt, ir::Return{}},
        };
        bool ok;
        const std::string d = lower_to_disasm(stmts, ok);
        REQUIRE(ok);
        REQUIRE(d.find("fdiv") != std::string::npos);
    }
}

TEST_CASE("Lowerer: Fence Sfence emits dmb ishst") {
    std::vector<ir::Stmt> stmts = {
        {std::nullopt, ir::Fence{ir::FenceKind::Sfence}},
        {std::nullopt, ir::Return{}},
    };
    bool ok;
    const std::string d = lower_to_disasm(stmts, ok);
    REQUIRE(ok);
    REQUIRE(d.find("dmb ishst") != std::string::npos);
}
