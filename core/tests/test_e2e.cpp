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
    // Lowering produces:
    //   mov x0, #0x2a     ; %0 = const 42 (scratch x0)
    //   mov x10, x0       ; storereg rax (rax → x10)
    //   ret               ; Return
    // That's 3 * 4 = 12 bytes.
    REQUIRE(bytes.size() == 12);

    std::uint32_t w[3];
    std::memcpy(w, bytes.data(), 12);

    // First word: movz x0, #42 (vixl picks movz for a 16-bit-fitting const).
    REQUIRE(w[0] == arm64::movz_x(arm64::Reg::X0, 42, 0).raw);
    // Last word: ret x30.
    REQUIRE(w[2] == arm64::ret().raw);
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
