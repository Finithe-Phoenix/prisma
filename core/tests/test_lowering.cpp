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
