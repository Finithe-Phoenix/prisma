// core/tests/test_e2e.cpp — end-to-end: x86 bytes → IR → ARM64 bytes.
//
// This is the "hello world" of the whole DBT: we take a real x86_64
// instruction, decode it to our IR, lower the IR to ARM64 through the
// emitter, and verify the bytes match a by-hand lowering of the same
// operation. The lowering used here is a trivial one-to-one (constants
// direct to registers via mov_imm64, RET to ret) — no register allocation,
// no optimisation. Fase 1-2 replaces this with the real lowering.

#include <catch2/catch_test_macros.hpp>
#include <cstring>
#include <variant>

#include "prisma/arm64_encoding.hpp"
#include "prisma/decoder.hpp"
#include "prisma/emitter.hpp"
#include "prisma/ir.hpp"

using namespace prisma;

namespace {

// Trivial lowering: walks a sequence of IR statements and emits ARM64.
// Only handles the subset produced by the MVP decoder. This is NOT the
// real backend — the real one lives in `core/src/backend/lowering.cpp`
// (Fase 1). This is here so the test suite can exercise the full pipe.
void lower_and_emit(const std::vector<ir::Stmt>& stmts, backend::Emitter& em) {
    // We build a trivial ref→uint64 map for Constant refs, so StoreReg can
    // look up the constant value and emit mov_imm64 into the mapped host reg.
    std::unordered_map<ir::Ref, std::uint64_t> const_values;

    for (const auto& s : stmts) {
        std::visit([&](auto const& op) {
            using T = std::decay_t<decltype(op)>;
            if constexpr (std::is_same_v<T, ir::Constant>) {
                REQUIRE(s.result.has_value());
                const_values[*s.result] = op.value;
            } else if constexpr (std::is_same_v<T, ir::StoreReg>) {
                // For MVP: assume the value came from a Constant we just saw.
                auto it = const_values.find(op.value);
                REQUIRE(it != const_values.end());
                em.mov_imm64(arm64::host_reg_for(op.reg), it->second);
            } else if constexpr (std::is_same_v<T, ir::Return>) {
                em.ret();
            }
            // LoadReg / BinOp are produced by ADD/SUB decoding but the
            // trivial lowering here doesn't implement them. The ADD/SUB
            // end-to-end test below is deliberately scoped to "decodes
            // the right IR"; emitting ADD x/y/z lowering is Fase 1 work.
        }, s.op);
    }
}

}  // namespace

TEST_CASE("e2e: MOV rax, 42 ; RET → ARM64 bytes match predicted encoding") {
    // x86_64 bytes for:
    //   48 B8 2A 00 00 00 00 00 00 00   mov rax, 42
    //   C3                              ret
    const std::vector<std::uint8_t> guest_bytes = {
        0x48, 0xB8, 0x2A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0xC3,
    };

    // 1. Decode the two instructions.
    ir::Ref next = 0;
    std::vector<ir::Stmt> all;
    std::size_t cursor = 0;
    while (cursor < guest_bytes.size()) {
        std::span<const std::uint8_t> remaining(
            guest_bytes.data() + cursor, guest_bytes.size() - cursor);
        auto r = decoder::decode_one(remaining, next);
        REQUIRE(std::holds_alternative<decoder::Decoded>(r));
        auto& d = std::get<decoder::Decoded>(r);
        for (auto& s : d.stmts) all.push_back(std::move(s));
        cursor += d.bytes_consumed;
    }

    // Sanity: three statements (const, storereg, ret).
    REQUIRE(all.size() == 3);

    // 2. Lower trivially to ARM64.
    backend::Emitter em;
    lower_and_emit(all, em);
    em.finalize();

    // 3. Check the emitted bytes. For imm=42 (fits in 16 bits),
    //    mov_imm64 should produce a single movz. host_reg_for(rax) = x10.
    const auto bytes = em.code_bytes();
    REQUIRE(bytes.size() == 8);  // movz + ret = 2 * 4 bytes

    std::uint32_t w0, w1;
    std::memcpy(&w0, bytes.data(),     4);
    std::memcpy(&w1, bytes.data() + 4, 4);

    // movz x10, #42, lsl #0
    REQUIRE(w0 == arm64::movz_x(arm64::Reg::X10, 42, 0).raw);
    // ret
    REQUIRE(w1 == arm64::ret().raw);
}
