// core/tests/test_e2e.cpp — end-to-end pipeline tests.
//
// Structural e2e:  x86 bytes → IR → (passes) → ARM64 bytes (byte-level
//                  comparison against hand-rolled encoders).
//
// Executing e2e:   x86 bytes → IR → Lowerer → JitBuffer → fn() → value.
//                  Only on ARM64 hosts.
//
// The first layer still exists as a sanity check; the second is the
// "Prisma actually translates and runs a program" milestone.

#include <catch2/catch_test_macros.hpp>
#include <cstdint>
#include <cstring>
#include <variant>
#include <vector>

#include "prisma/arm64_encoding.hpp"
#include "prisma/decoder.hpp"
#include "prisma/emitter.hpp"
#include "prisma/ir.hpp"
#include "prisma/jit_memory.hpp"
#include "prisma/lowering.hpp"
#include "prisma/passes.hpp"
#include "prisma/translator.hpp"

using namespace prisma;

namespace {

constexpr bool is_arm64 =
#if defined(__aarch64__) || defined(__arm64__)
    true;
#else
    false;
#endif

// Decode a whole guest byte stream into a flat statement list. Tests
// assume the input is self-contained and decodes cleanly.
std::vector<ir::Stmt> decode_all(std::span<const std::uint8_t> guest_bytes) {
    ir::Ref next = 0;
    std::vector<ir::Stmt> stmts;
    std::size_t cursor = 0;
    while (cursor < guest_bytes.size()) {
        auto res = decoder::decode_one(
            guest_bytes.subspan(cursor), next);
        REQUIRE(std::holds_alternative<decoder::Decoded>(res));
        auto& d = std::get<decoder::Decoded>(res);
        for (auto& s : d.stmts) stmts.push_back(std::move(s));
        cursor += d.bytes_consumed;
    }
    return stmts;
}

}  // namespace

// ---------------------------------------------------------------------------
// Structural e2e: byte-level verification (host-agnostic).
// ---------------------------------------------------------------------------

TEST_CASE("e2e structural: MOV rax, 42 ; RET lowered to specific ARM64 bytes") {
    const std::vector<std::uint8_t> guest_bytes = {
        0x48, 0xB8, 0x2A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0xC3,
    };
    auto stmts = decode_all(guest_bytes);

    backend::Emitter em;
    backend::Lowerer lw(em);
    auto r = lw.lower(stmts);
    REQUIRE(r.success);
    em.finalize();

    const auto bytes = em.code_bytes();
    // Lowering produces (post-F1-RT-004 halt-sentinel convention):
    //   mov x0, #0x2a     ; %0 = const 42 (scratch x0)
    //   mov x10, x0       ; storereg rax (rax → x10)
    //   mov x0, #0        ; IR::Return emits halt sentinel in x0
    //   ret
    // That's 4 * 4 = 16 bytes.
    REQUIRE(bytes.size() == 16);

    std::uint32_t w[4];
    std::memcpy(w, bytes.data(), 16);

    // First word: movz x0, #42.
    REQUIRE(w[0] == arm64::movz_x(arm64::Reg::X0, 42, 0).raw);
    // Third word: movz x0, #0 (halt sentinel).
    REQUIRE(w[2] == arm64::movz_x(arm64::Reg::X0, 0, 0).raw);
    // Fourth word: ret x30.
    REQUIRE(w[3] == arm64::ret().raw);
}

// ---------------------------------------------------------------------------
// Executing e2e (ARM64 hosts only).
// ---------------------------------------------------------------------------
//
// The IR-translated body leaves the "return value" in the host register
// mapped to guest rax (x10 per our fixed mapping). To test from a C
// caller we wrap the body in a tiny thunk that moves x10 → x0 before
// returning, so AAPCS64 sees the value.

TEST_CASE("e2e executing: translate and run `MOV rax, 42 ; RET`",
          "[arm64-only]") {
    if constexpr (!is_arm64) {
        SUCCEED("skipped on non-ARM64 host");
        return;
    }

    const std::vector<std::uint8_t> guest_bytes = {
        0x48, 0xB8, 0x2A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0xC3,
    };
    auto stmts = decode_all(guest_bytes);

    // Optional but realistic: run the pipeline before lowering.
    stmts = passes::constant_propagate(stmts);
    stmts = passes::dead_code_eliminate(stmts);

    // Lower everything *except* the trailing Return — we want to emit
    // our own epilogue that moves x10 → x0 (AAPCS64 return) before ret.
    REQUIRE_FALSE(stmts.empty());
    REQUIRE(std::holds_alternative<ir::Return>(stmts.back().op));
    const std::span<const ir::Stmt> body{stmts.data(), stmts.size() - 1};

    backend::Emitter em;
    backend::Lowerer lw(em);
    auto r = lw.lower(body);
    REQUIRE(r.success);

    // AAPCS64 epilogue: move guest rax's host register into x0, then ret.
    em.mov_reg_reg(arm64::Reg::X0, arm64::host_reg_for(ir::Gpr::Rax));
    em.ret();
    em.finalize();

    runtime::JitBuffer jit(em.code_bytes().size());
    REQUIRE(jit.write(em.code_bytes()));
    jit.make_executable();

    using Fn = std::uint64_t (*)();
    Fn fn = reinterpret_cast<Fn>(const_cast<std::uint8_t*>(jit.entry()));
    REQUIRE(fn() == 42u);
}

TEST_CASE("e2e executing: translate and run `MOV rax, 100 ; MOV rbx, 42 ; XOR rax, rbx ; RET` (= 78)",
          "[arm64-only]") {
    if constexpr (!is_arm64) {
        SUCCEED("skipped on non-ARM64 host");
        return;
    }

    // Build the guest program explicitly:
    //   48 B8 64 00 00 00 00 00 00 00   mov rax, 100
    //   48 BB 2A 00 00 00 00 00 00 00   mov rbx, 42
    //   48 31 D8                         xor rax, rbx        ; rax := 100^42 = 78
    //   C3                               ret
    const std::vector<std::uint8_t> guest_bytes = {
        0x48, 0xB8, 0x64, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x48, 0xBB, 0x2A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x48, 0x31, 0xD8,
        0xC3,
    };
    auto stmts = decode_all(guest_bytes);

    backend::Emitter em;
    backend::Lowerer lw(em);
    const std::span<const ir::Stmt> body{stmts.data(), stmts.size() - 1};
    auto r = lw.lower(body);
    REQUIRE(r.success);

    em.mov_reg_reg(arm64::Reg::X0, arm64::host_reg_for(ir::Gpr::Rax));
    em.ret();
    em.finalize();

    runtime::JitBuffer jit(em.code_bytes().size());
    REQUIRE(jit.write(em.code_bytes()));
    jit.make_executable();

    using Fn = std::uint64_t (*)();
    Fn fn = reinterpret_cast<Fn>(const_cast<std::uint8_t*>(jit.entry()));
    REQUIRE(fn() == (100u ^ 42u));  // 78
}

// ---------------------------------------------------------------------------
// Memory round-trips (ARM64 only).
// ---------------------------------------------------------------------------

TEST_CASE("e2e executing: load from guest memory returns stored value",
          "[arm64-only]") {
    if constexpr (!is_arm64) {
        SUCCEED("skipped on non-ARM64 host");
        return;
    }

    // IR: %0 = loadreg rbx        ; rbx holds the address
    //     %1 = load.tso.i64 [%0]
    //          storereg rax, %1
    //          ret
    std::vector<ir::Stmt> stmts = {
        {0u, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}},
        {1u, ir::LoadMemTSO{0u, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 1u, ir::OpSize::I64}},
        {std::nullopt, ir::Return{}},
    };

    backend::Emitter em;

    // Thunk prologue: AAPCS64 passes the first arg in x0. Copy it into
    // rbx's pinned host register (x13) so LoadReg rbx reads the right
    // value.
    em.mov_reg_reg(arm64::host_reg_for(ir::Gpr::Rbx), arm64::Reg::X0);

    // Lower everything except the Return; we'll emit our own epilogue.
    backend::Lowerer lw(em);
    auto r = lw.lower({stmts.data(), stmts.size() - 1});
    REQUIRE(r.success);

    // Epilogue: return guest rax through AAPCS64 x0.
    em.mov_reg_reg(arm64::Reg::X0, arm64::host_reg_for(ir::Gpr::Rax));
    em.ret();
    em.finalize();

    // Host-side backing memory the JIT'd code will read.
    alignas(8) std::uint64_t backing = 0xBADC0FFEE0DDF00DULL;

    runtime::JitBuffer jit(em.code_bytes().size());
    REQUIRE(jit.write(em.code_bytes()));
    jit.make_executable();

    using Fn = std::uint64_t (*)(std::uint64_t*);
    Fn fn = reinterpret_cast<Fn>(const_cast<std::uint8_t*>(jit.entry()));
    REQUIRE(fn(&backing) == 0xBADC0FFEE0DDF00DULL);
}

TEST_CASE("e2e executing: store then re-load round-trips through guest memory",
          "[arm64-only]") {
    if constexpr (!is_arm64) {
        SUCCEED("skipped on non-ARM64 host");
        return;
    }

    // Three-step IR:
    //   %0 = loadreg rbx          ; address passed in rbx
    //   %1 = loadreg rax          ; value passed in rax
    //        storemem.tso.i64 [%0], %1
    //   %2 = load.tso.i64 [%0]    ; read back
    //        storereg rcx, %2     ; stash into rcx; thunk will return rcx
    //        ret
    //
    // The thunk uses two AAPCS64 args (x0 → rbx, x1 → rax) and returns
    // rcx so we can verify the whole round-trip.
    std::vector<ir::Stmt> stmts = {
        {0u, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}},
        {1u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {std::nullopt, ir::StoreMemTSO{0u, 1u, ir::OpSize::I64}},
        {2u, ir::LoadMemTSO{0u, ir::OpSize::I64}},
        {std::nullopt, ir::StoreReg{ir::Gpr::Rcx, 2u, ir::OpSize::I64}},
        {std::nullopt, ir::Return{}},
    };

    backend::Emitter em;

    // Prologue: x13 ← x0 (rbx = addr), x10 ← x1 (rax = value).
    em.mov_reg_reg(arm64::host_reg_for(ir::Gpr::Rbx), arm64::Reg::X0);
    em.mov_reg_reg(arm64::host_reg_for(ir::Gpr::Rax), arm64::Reg::X1);

    backend::Lowerer lw(em);
    auto r = lw.lower({stmts.data(), stmts.size() - 1});
    REQUIRE(r.success);

    // Epilogue: return rcx via x0.
    em.mov_reg_reg(arm64::Reg::X0, arm64::host_reg_for(ir::Gpr::Rcx));
    em.ret();
    em.finalize();

    alignas(8) std::uint64_t backing = 0;

    runtime::JitBuffer jit(em.code_bytes().size());
    REQUIRE(jit.write(em.code_bytes()));
    jit.make_executable();

    using Fn = std::uint64_t (*)(std::uint64_t*, std::uint64_t);
    Fn fn = reinterpret_cast<Fn>(const_cast<std::uint8_t*>(jit.entry()));

    constexpr std::uint64_t payload = 0xCAFEF00DDEADBEEFULL;
    REQUIRE(fn(&backing, payload) == payload);
    REQUIRE(backing == payload);  // the store also landed in host memory.
}

// ---------------------------------------------------------------------------
// Control flow e2e — CMP + Jcc return the correct next-PC in x0.
// ---------------------------------------------------------------------------

TEST_CASE("e2e executing: CMP + JE picks the taken target when operands are equal",
          "[arm64-only]") {
    if constexpr (!is_arm64) {
        SUCCEED("skipped on non-ARM64 host");
        return;
    }

    // Build IR directly (we test the decoder path separately):
    //   %0 = loadreg rax
    //   %1 = loadreg rbx
    //          cmpflags %0, %1        ; sets NZCV
    //          condjmprel.eq 0xDEAD, 0xBEEF
    // The thunk supplies rax via x0, rbx via x1.
    const std::uint64_t taken        = 0x00000000DEADDEADULL;
    const std::uint64_t fallthrough  = 0x00000000BEEFBEEFULL;

    std::vector<ir::Stmt> stmts = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}},
        {std::nullopt, ir::CmpFlags{0u, 1u, ir::OpSize::I64}},
        {std::nullopt, ir::CondJumpRel{ir::CondCode::Eq, taken, fallthrough}},
    };

    backend::Emitter em;
    // Prologue: x10 (rax) ← x0, x13 (rbx) ← x1.
    em.mov_reg_reg(arm64::host_reg_for(ir::Gpr::Rax), arm64::Reg::X0);
    em.mov_reg_reg(arm64::host_reg_for(ir::Gpr::Rbx), arm64::Reg::X1);
    backend::Lowerer lw(em);
    REQUIRE(lw.lower(stmts).success);
    // No epilogue needed: CondJumpRel already ends with ret.
    em.finalize();

    runtime::JitBuffer jit(em.code_bytes().size());
    REQUIRE(jit.write(em.code_bytes()));
    jit.make_executable();

    using Fn = std::uint64_t (*)(std::uint64_t, std::uint64_t);
    Fn fn = reinterpret_cast<Fn>(const_cast<std::uint8_t*>(jit.entry()));

    // Equal operands → taken branch.
    REQUIRE(fn(42, 42) == taken);
    // Unequal → fallthrough.
    REQUIRE(fn(42, 43) == fallthrough);
    // Still equal but with different operand values — semantics match.
    REQUIRE(fn(0, 0)   == taken);
}

TEST_CASE("e2e executing: CMP + JNE is symmetric to CMP + JE",
          "[arm64-only]") {
    if constexpr (!is_arm64) {
        SUCCEED("skipped on non-ARM64 host");
        return;
    }

    const std::uint64_t taken       = 0xAAAAAAAAAAAAAAAAULL;
    const std::uint64_t fallthrough = 0xBBBBBBBBBBBBBBBBULL;

    std::vector<ir::Stmt> stmts = {
        {0u, ir::LoadReg{ir::Gpr::Rax, ir::OpSize::I64}},
        {1u, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}},
        {std::nullopt, ir::CmpFlags{0u, 1u, ir::OpSize::I64}},
        {std::nullopt, ir::CondJumpRel{ir::CondCode::Ne, taken, fallthrough}},
    };

    backend::Emitter em;
    em.mov_reg_reg(arm64::host_reg_for(ir::Gpr::Rax), arm64::Reg::X0);
    em.mov_reg_reg(arm64::host_reg_for(ir::Gpr::Rbx), arm64::Reg::X1);
    backend::Lowerer lw(em);
    REQUIRE(lw.lower(stmts).success);
    em.finalize();

    runtime::JitBuffer jit(em.code_bytes().size());
    REQUIRE(jit.write(em.code_bytes()));
    jit.make_executable();

    using Fn = std::uint64_t (*)(std::uint64_t, std::uint64_t);
    Fn fn = reinterpret_cast<Fn>(const_cast<std::uint8_t*>(jit.entry()));

    REQUIRE(fn(1, 2) == taken);         // unequal → JNE taken.
    REQUIRE(fn(7, 7) == fallthrough);   // equal   → fallthrough.
}

TEST_CASE("e2e executing: unconditional JumpRel returns its target",
          "[arm64-only]") {
    if constexpr (!is_arm64) {
        SUCCEED("skipped on non-ARM64 host");
        return;
    }

    const std::uint64_t target = 0xFEEDFACECAFEBABEULL;
    std::vector<ir::Stmt> stmts = {
        {std::nullopt, ir::JumpRel{target}},
    };

    backend::Emitter em;
    backend::Lowerer lw(em);
    REQUIRE(lw.lower(stmts).success);
    em.finalize();

    runtime::JitBuffer jit(em.code_bytes().size());
    REQUIRE(jit.write(em.code_bytes()));
    jit.make_executable();

    using Fn = std::uint64_t (*)();
    Fn fn = reinterpret_cast<Fn>(const_cast<std::uint8_t*>(jit.entry()));
    REQUIRE(fn() == target);
}

TEST_CASE("e2e executing: Translator end-to-end for decoded CMP + JE sequence",
          "[arm64-only]") {
    if constexpr (!is_arm64) {
        SUCCEED("skipped on non-ARM64 host");
        return;
    }

    // Guest program at 0x8000:
    //   48 B8 2A 00 00 00 00 00 00 00   mov rax, 42       ; 10 bytes
    //   48 BB 2A 00 00 00 00 00 00 00   mov rbx, 42       ; 10 bytes
    //   48 39 D8                        cmp rax, rbx      ; 3  bytes
    //   74 10                           je +16            ; 2  bytes
    //
    // Addresses: cmp ends at 0x8017, je ends at 0x8019, taken = 0x8029,
    // fallthrough = 0x8019. Since rax == rbx, the Translator'd block
    // should return 0x8029.
    const std::vector<std::uint8_t> guest_bytes = {
        0x48, 0xB8, 0x2A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x48, 0xBB, 0x2A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x48, 0x39, 0xD8,
        0x74, 0x10,
    };

    translator::Translator t;
    auto r = t.translate(0x8000, guest_bytes);
    REQUIRE(std::holds_alternative<translator::TranslatedBlock>(r));
    const auto& b = std::get<translator::TranslatedBlock>(r);

    using Fn = std::uint64_t (*)();
    Fn fn = reinterpret_cast<Fn>(const_cast<std::uint8_t*>(b.code_entry));
    REQUIRE(fn() == 0x8029u);  // taken branch PC.
}

TEST_CASE("e2e executing: decoded x86 `mov rax, [rbx]` JITs and runs",
          "[arm64-only]") {
    if constexpr (!is_arm64) {
        SUCCEED("skipped on non-ARM64 host");
        return;
    }

    // This is the full pipeline: x86 bytes, decoded via the memory-operand
    // path of the decoder, optimised, lowered, JIT'd, executed.
    //
    //   48 8B 03   →   mov rax, qword ptr [rbx]
    //   C3         →   ret
    const std::vector<std::uint8_t> guest_bytes = {
        0x48, 0x8B, 0x03,
        0xC3,
    };
    auto stmts = decode_all(guest_bytes);

    // Run the default pass pipeline — correctness must be invariant.
    auto [opt, _stats] = passes::default_pipeline().run(stmts);

    backend::Emitter em;
    em.mov_reg_reg(arm64::host_reg_for(ir::Gpr::Rbx), arm64::Reg::X0);

    backend::Lowerer lw(em);
    // Drop the trailing Return so we can emit our own epilogue.
    REQUIRE_FALSE(opt.empty());
    REQUIRE(std::holds_alternative<ir::Return>(opt.back().op));
    auto r = lw.lower({opt.data(), opt.size() - 1});
    REQUIRE(r.success);

    em.mov_reg_reg(arm64::Reg::X0, arm64::host_reg_for(ir::Gpr::Rax));
    em.ret();
    em.finalize();

    alignas(8) std::uint64_t backing = 0x1234'5678'9ABC'DEF0ULL;
    runtime::JitBuffer jit(em.code_bytes().size());
    REQUIRE(jit.write(em.code_bytes()));
    jit.make_executable();

    using Fn = std::uint64_t (*)(std::uint64_t*);
    Fn fn = reinterpret_cast<Fn>(const_cast<std::uint8_t*>(jit.entry()));
    REQUIRE(fn(&backing) == 0x1234'5678'9ABC'DEF0ULL);
}

TEST_CASE("e2e executing: i8/i16/i32/i64 loads each see the right width",
          "[arm64-only]") {
    if constexpr (!is_arm64) {
        SUCCEED("skipped on non-ARM64 host");
        return;
    }

    // The payload is carefully chosen so that the result of each width
    // differs. Little-endian: the low 8 bits are 0x11, low 16 are 0x2211,
    // low 32 are 0x44332211, full 64 are the whole thing.
    alignas(8) std::uint64_t backing = 0x8877665544332211ULL;

    auto translate_and_run = [&](ir::OpSize sz) -> std::uint64_t {
        std::vector<ir::Stmt> stmts = {
            {0u, ir::LoadReg{ir::Gpr::Rbx, ir::OpSize::I64}},
            {1u, ir::LoadMem{0u, sz}},            // non-TSO so we exercise ldr*
            {std::nullopt, ir::StoreReg{ir::Gpr::Rax, 1u, ir::OpSize::I64}},
        };

        backend::Emitter em;
        em.mov_reg_reg(arm64::host_reg_for(ir::Gpr::Rbx), arm64::Reg::X0);
        backend::Lowerer lw(em);
        REQUIRE(lw.lower(stmts).success);
        em.mov_reg_reg(arm64::Reg::X0, arm64::host_reg_for(ir::Gpr::Rax));
        em.ret();
        em.finalize();

        runtime::JitBuffer jit(em.code_bytes().size());
        REQUIRE(jit.write(em.code_bytes()));
        jit.make_executable();

        using Fn = std::uint64_t (*)(std::uint64_t*);
        Fn fn = reinterpret_cast<Fn>(const_cast<std::uint8_t*>(jit.entry()));
        return fn(&backing);
    };

    REQUIRE(translate_and_run(ir::OpSize::I8)  == 0x11u);
    REQUIRE(translate_and_run(ir::OpSize::I16) == 0x2211u);
    REQUIRE(translate_and_run(ir::OpSize::I32) == 0x44332211ULL);
    REQUIRE(translate_and_run(ir::OpSize::I64) == 0x8877665544332211ULL);
}

TEST_CASE("e2e executing: passes fold constants — output matches unoptimised",
          "[arm64-only]") {
    // This test establishes a property we'll want to keep as passes grow:
    // "the pipeline's observable output does not depend on whether passes
    // ran". For our current passes (const_prop + dce), both paths should
    // return the same value.
    if constexpr (!is_arm64) {
        SUCCEED("skipped on non-ARM64 host");
        return;
    }

    const std::vector<std::uint8_t> guest_bytes = {
        0x48, 0xB8, 0x0A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  // mov rax, 10
        0x48, 0xBB, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,  // mov rbx, 32
        0x48, 0x01, 0xD8,                                             // add rax, rbx → 42
        0xC3,                                                         // ret
    };

    auto run = [&](std::vector<ir::Stmt> stmts) {
        backend::Emitter em;
        backend::Lowerer lw(em);
        const std::span<const ir::Stmt> body{stmts.data(), stmts.size() - 1};
        REQUIRE(lw.lower(body).success);
        em.mov_reg_reg(arm64::Reg::X0, arm64::host_reg_for(ir::Gpr::Rax));
        em.ret();
        em.finalize();

        runtime::JitBuffer jit(em.code_bytes().size());
        REQUIRE(jit.write(em.code_bytes()));
        jit.make_executable();

        using Fn = std::uint64_t (*)();
        Fn fn = reinterpret_cast<Fn>(const_cast<std::uint8_t*>(jit.entry()));
        return fn();
    };

    auto raw = decode_all(guest_bytes);
    auto optimised = passes::dead_code_eliminate(passes::constant_propagate(raw));

    const std::uint64_t v_raw = run(raw);
    const std::uint64_t v_opt = run(optimised);

    REQUIRE(v_raw == 42u);
    REQUIRE(v_opt == 42u);
    REQUIRE(v_raw == v_opt);
}
