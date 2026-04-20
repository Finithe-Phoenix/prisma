// core/tests/test_benchmarks.cpp — F1-TC-007 perf regression harness.
//
// Uses Catch2's built-in BENCHMARK macro. Each case is tagged with
// [.benchmark] so Catch2 skips it in normal test runs — they only
// fire with `--run-benchmarks` (or by selecting the tag explicitly).
//
// Usage:
//   ./prisma_core_tests --run-benchmarks
//   ./prisma_core_tests "[.benchmark]"
//
// These numbers are noisy on a macbook with spotlight etc. running.
// Treat them as order-of-magnitude sanity checks — a 5x regression is
// real, a 20% shift is noise.

#include <catch2/catch_test_macros.hpp>
#include <catch2/benchmark/catch_benchmark.hpp>

#include <cstdint>
#include <random>
#include <vector>

#include "prisma/ir.hpp"
#include "prisma/passes.hpp"
#include "prisma/sha256.hpp"
#include "prisma/translation_cache.hpp"

using namespace prisma;

// ---------------------------------------------------------------------------
// Representative workloads — kept small enough to be quick, large enough
// that per-op overhead doesn't dominate.
// ---------------------------------------------------------------------------

namespace {

// Build a stmt list with `n` redundant Add pairs — exercises CSE, DCE,
// const_prop in proportion.
std::vector<ir::Stmt> build_pass_workload(unsigned n) {
    std::vector<ir::Stmt> s;
    s.reserve(n * 3 + 2);
    s.push_back({0u, ir::Constant{10, ir::OpSize::I64}});
    s.push_back({1u, ir::Constant{20, ir::OpSize::I64}});
    ir::Ref next = 2;
    for (unsigned i = 0; i < n; ++i) {
        s.push_back({next, ir::BinOp{ir::BinOpKind::Add, 0u, 1u, ir::OpSize::I64}});
        ++next;
    }
    s.push_back({std::nullopt,
                 ir::StoreReg{ir::Gpr::Rax, next - 1, ir::OpSize::I64}});
    return s;
}

std::vector<std::uint8_t> random_bytes(std::size_t n, std::uint64_t seed) {
    std::mt19937_64 rng(seed);
    std::vector<std::uint8_t> out(n);
    for (std::size_t i = 0; i < n; ++i) {
        out[i] = static_cast<std::uint8_t>(rng() & 0xFF);
    }
    return out;
}

}  // namespace

// ---------------------------------------------------------------------------
// Pass pipeline throughput
// ---------------------------------------------------------------------------

TEST_CASE("benchmark: default_pipeline on 100 redundant BinOps",
          "[.benchmark]") {
    auto input = build_pass_workload(100);
    auto pm = passes::default_pipeline();
    BENCHMARK("default_pipeline (100 ops)") {
        auto [out, _stats] = pm.run(input);
        return out.size();
    };
}

TEST_CASE("benchmark: const_prop pass in isolation",
          "[.benchmark]") {
    auto input = build_pass_workload(100);
    BENCHMARK("const_prop (100 ops)") {
        return passes::constant_propagate(input).size();
    };
}

TEST_CASE("benchmark: dce pass in isolation",
          "[.benchmark]") {
    auto input = build_pass_workload(100);
    BENCHMARK("dce (100 ops)") {
        return passes::dead_code_eliminate(input).size();
    };
}

// ---------------------------------------------------------------------------
// Hash throughput
// ---------------------------------------------------------------------------

TEST_CASE("benchmark: FNV-1a over 1 KiB", "[.benchmark]") {
    auto buf = random_bytes(1024, 0xFEEDu);
    BENCHMARK("fnv1a_64 (1 KiB)") {
        return cache::fnv1a_64(buf);
    };
}

TEST_CASE("benchmark: SHA-256 over 1 KiB", "[.benchmark]") {
    auto buf = random_bytes(1024, 0xFEEDu);
    BENCHMARK("sha256 (1 KiB)") {
        return cache::sha256(buf);
    };
}

// ---------------------------------------------------------------------------
// Cache throughput
// ---------------------------------------------------------------------------

TEST_CASE("benchmark: TranslationCache lookup hit path", "[.benchmark]") {
    cache::TranslationCache tc;
    // Seed 1000 entries.
    std::vector<std::vector<std::uint8_t>> bodies;
    bodies.reserve(1000);
    for (std::uint64_t i = 0; i < 1000; ++i) {
        auto body = random_bytes(32, i);
        const std::uint64_t h = cache::fnv1a_64(body);
        cache::Entry e{std::vector<std::uint8_t>(16, 0), body.size(), h};
        tc.insert(cache::Key{i, h}, std::move(e));
        bodies.push_back(std::move(body));
    }
    std::uint64_t idx = 0;
    BENCHMARK("cache lookup (round-robin 1000-entry)") {
        const auto& b = bodies[idx % bodies.size()];
        auto r = tc.lookup(idx % bodies.size(), b);
        ++idx;
        return std::holds_alternative<const cache::Entry*>(r) ? 1 : 0;
    };
}
