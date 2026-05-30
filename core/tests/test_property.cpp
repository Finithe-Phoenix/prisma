// core/tests/test_property.cpp — F1-TC-010 property-based tests for
// IR passes + serialization. Uses Catch2's GENERATE + a small custom
// random-stmt generator to avoid pulling rapidcheck. The seed is
// fixed-per-shard so failures are reproducible.

#include <catch2/catch_test_macros.hpp>
#include <catch2/generators/catch_generators.hpp>

#include <cstdint>
#include <random>
#include <vector>

#include "prisma/ir.hpp"
#include "prisma/ir_serialize.hpp"
#include "prisma/ir_validate.hpp"
#include "prisma/passes.hpp"
#include "prisma/profiler.hpp"

using namespace prisma;

namespace {

// Random IR generator. Produces a stmt list that the validator
// considers well-formed. Sizes / opcodes are drawn uniformly from
// the supported subsets.
struct Rng {
    std::mt19937_64 e;
    std::uniform_int_distribution<int> dist_size{0, 3};       // OpSize
    std::uniform_int_distribution<int> dist_binop{0, 12};     // BinOpKind
    std::uniform_int_distribution<int> dist_cc{0, 9};         // first 10 CondCodes
    std::uniform_int_distribution<int> dist_gpr{0, 15};
    std::uniform_int_distribution<std::uint64_t> dist_imm{0, ~0ULL};

    explicit Rng(std::uint64_t seed) : e(seed) {}

    ir::OpSize size() {
        return static_cast<ir::OpSize>(dist_size(e));
    }
    ir::BinOpKind binop() {
        return static_cast<ir::BinOpKind>(dist_binop(e));
    }
    ir::CondCode cc() {
        return static_cast<ir::CondCode>(dist_cc(e));
    }
    ir::Gpr gpr() {
        return static_cast<ir::Gpr>(dist_gpr(e));
    }
    std::uint64_t imm() { return dist_imm(e); }
};

// Build a well-typed program of `n` statements using `rng`. The
// statements form a valid SSA program (every operand ref is defined
// earlier; pure/impure shape is correct; sizes match).
std::vector<ir::Stmt> random_program(Rng& rng, std::size_t n) {
    std::vector<ir::Stmt> stmts;
    stmts.reserve(n);
    ir::Ref next = 0;

    // Always start with at least one Constant so subsequent BinOps
    // have something to read.
    const auto sz0 = rng.size();
    const ir::Ref r0 = next++;
    stmts.push_back({r0, ir::Constant{rng.imm(), sz0}});
    // Track a list of (ref, size) for size-consistent reads.
    std::vector<std::pair<ir::Ref, ir::OpSize>> defs;
    defs.push_back({r0, sz0});

    for (std::size_t i = 1; i < n; ++i) {
        // Pick a category. Keep distribution biased to ALU since
        // that's where the optimiser does most work.
        std::uniform_int_distribution<int> dcat(0, 4);
        switch (dcat(rng.e)) {
            case 0: {  // another Constant
                const auto sz = rng.size();
                const ir::Ref r = next++;
                stmts.push_back({r, ir::Constant{rng.imm(), sz}});
                defs.push_back({r, sz});
                break;
            }
            case 1: case 2: case 3: {  // BinOp on two same-size operands
                // Find any two refs with the same size; if none yet,
                // emit a fresh Constant pair.
                std::vector<std::size_t> by_size[4];
                for (std::size_t k = 0; k < defs.size(); ++k) {
                    by_size[static_cast<int>(defs[k].second)].push_back(k);
                }
                int picked_sz = -1;
                for (int s = 0; s < 4; ++s) {
                    if (by_size[s].size() >= 2) { picked_sz = s; break; }
                }
                if (picked_sz < 0) {
                    const auto sz = rng.size();
                    const ir::Ref r = next++;
                    stmts.push_back({r, ir::Constant{rng.imm(), sz}});
                    defs.push_back({r, sz});
                    break;
                }
                const auto& pool = by_size[picked_sz];
                std::uniform_int_distribution<std::size_t> idx(0, pool.size() - 1);
                const ir::Ref lhs = defs[pool[idx(rng.e)]].first;
                const ir::Ref rhs = defs[pool[idx(rng.e)]].first;
                const auto sz = static_cast<ir::OpSize>(picked_sz);
                const ir::Ref r = next++;
                stmts.push_back({r,
                    ir::BinOp{rng.binop(), lhs, rhs, sz}});
                defs.push_back({r, sz});
                break;
            }
            case 4: {  // StoreReg of an existing ref
                std::uniform_int_distribution<std::size_t> idx(0, defs.size() - 1);
                const auto [ref, sz] = defs[idx(rng.e)];
                stmts.push_back({std::nullopt,
                    ir::StoreReg{rng.gpr(), ref, sz}});
                break;
            }
        }
    }
    return stmts;
}

}  // namespace

// ---------------------------------------------------------------------
// Property: every well-typed random program serialises and deserialises
// without loss.
// ---------------------------------------------------------------------

TEST_CASE("property: random IR programs round-trip through the binary serializer",
          "[property][serialize]") {
    const std::uint64_t seed = GENERATE(0xCAFE'BABEull, 0xDEAD'BEEFull,
                                         0x1234'5678ull, 0x55AA'3CC3ull);
    Rng rng{seed};
    const std::size_t n = 50;
    auto program = random_program(rng, n);

    INFO("seed=0x" << std::hex << seed);
    auto bytes = ir::serialize(program);
    auto r     = ir::deserialize_stmts(bytes);
    REQUIRE(r.error == ir::DeserializeError::Ok);
    REQUIRE(r.stmts.size() == program.size());
    for (std::size_t i = 0; i < program.size(); ++i) {
        REQUIRE(r.stmts[i] == program[i]);
    }
}

// ---------------------------------------------------------------------
// Property: random programs validate (the generator only emits
// well-typed, def-before-use SSA).
// ---------------------------------------------------------------------

TEST_CASE("property: random IR programs are accepted by the validator",
          "[property][validate]") {
    const std::uint64_t seed = GENERATE(0xCAFE'BABEull, 0xDEAD'BEEFull);
    Rng rng{seed};
    const std::size_t n = 100;
    auto program = random_program(rng, n);

    INFO("seed=0x" << std::hex << seed);
    auto v = ir::validate(program);
    REQUIRE(v.ok);
}

// ---------------------------------------------------------------------
// Property: default_pipeline is idempotent (running it twice yields
// the same fixed point as running it once).
// ---------------------------------------------------------------------

TEST_CASE("property: default_pipeline is idempotent on random programs",
          "[property][passes]") {
    const std::uint64_t seed = GENERATE(0xAAAA'5555ull, 0x9876'5432ull);
    Rng rng{seed};
    auto program = random_program(rng, 30);

    INFO("seed=0x" << std::hex << seed);
    auto pm = passes::default_pipeline();
    auto [once, _s1]  = pm.run(program);
    auto [twice, _s2] = pm.run(once);
    REQUIRE(once == twice);
}

// ---------------------------------------------------------------------
// Property: passes never grow the stmt count beyond the input.
// ---------------------------------------------------------------------

TEST_CASE("property: default_pipeline never grows the program",
          "[property][passes]") {
    const std::uint64_t seed = GENERATE(0xF00D'BAADull, 0x1357'9BDFull);
    Rng rng{seed};
    auto program = random_program(rng, 75);

    INFO("seed=0x" << std::hex << seed);
    auto pm = passes::default_pipeline();
    auto [out, _stats] = pm.run(program);
    REQUIRE(out.size() <= program.size());
}

// ---------------------------------------------------------------------
// Property: OpCounter::total() equals the input stmt count.
// ---------------------------------------------------------------------

TEST_CASE("property: OpCounter total matches stmt count for random programs",
          "[property][profiler]") {
    const std::uint64_t seed = GENERATE(0xC0DE'FACEull);
    Rng rng{seed};
    auto program = random_program(rng, 200);

    ir::OpCounter c;
    c.visit(std::span<const ir::Stmt>(program));
    REQUIRE(c.total() == program.size());
}
