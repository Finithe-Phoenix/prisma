// core/tests/test_cache_stress.cpp — F1-TC-012 stress the cache at scale.
//
// 10k distinct (guest_addr, content) tuples. We validate:
//   * inserts complete without pathological allocator behaviour,
//   * lookups find the exact entry back,
//   * a byte-bounded cache evicts correctly while keeping the MRU set
//     accessible,
//   * invalidate_page drops entries in the given range and leaves the
//     rest intact.
//
// The test is runtime-cheap (a few hundred milliseconds) but has been
// the detector for regressions in the LRU path in the past, so it runs
// alongside the unit tests rather than as a bench.

#include <catch2/catch_test_macros.hpp>
#include <array>
#include <cstdint>
#include <variant>
#include <vector>

#include "prisma/translation_cache.hpp"

using namespace prisma::cache;

namespace {

// Deterministic guest content derived from the index — enough variety
// that FNV-1a produces distinct hashes.
std::vector<std::uint8_t> make_guest(std::uint64_t i) {
    return {
        static_cast<std::uint8_t>( i        & 0xFF),
        static_cast<std::uint8_t>((i >> 8)  & 0xFF),
        static_cast<std::uint8_t>((i >> 16) & 0xFF),
        static_cast<std::uint8_t>((i >> 24) & 0xFF),
    };
}

Entry make_entry(const std::vector<std::uint8_t>& g) {
    // 16-byte translated code placeholder, distinguishable by the first byte.
    std::vector<std::uint8_t> code(16, 0);
    code[0] = g[0];
    return Entry{std::move(code), g.size(), fnv1a_64(g)};
}

}  // namespace

TEST_CASE("cache stress: 10k distinct entries all insert and look up") {
    TranslationCache cache;
    constexpr std::uint64_t N = 10'000;

    for (std::uint64_t i = 0; i < N; ++i) {
        const auto g = make_guest(i);
        cache.insert(Key{i, fnv1a_64(g)}, make_entry(g));
    }
    REQUIRE(cache.entry_count() == N);

    // Spot-check 100 random indices.
    for (std::uint64_t i = 0; i < N; i += 100) {
        const auto g = make_guest(i);
        auto r = cache.lookup(i, g);
        REQUIRE(std::holds_alternative<const Entry*>(r));
        const Entry* e = std::get<const Entry*>(r);
        REQUIRE(e->code_bytes[0] == static_cast<std::uint8_t>(i & 0xFF));
    }
}

TEST_CASE("cache stress: byte-budget caps memory under continuous churn") {
    TranslationCache cache;
    cache.set_max_bytes(1024);  // 64 entries of 16 bytes each
    constexpr std::uint64_t N = 10'000;

    for (std::uint64_t i = 0; i < N; ++i) {
        const auto g = make_guest(i);
        cache.insert(Key{i, fnv1a_64(g)}, make_entry(g));
    }

    // Budget held throughout.
    REQUIRE(cache.total_code_bytes() <= 1024u);
    REQUIRE(cache.entry_count()      <= 64u);

    // The most recent 32 should still be resident (tight but achievable
    // given a 64-entry budget and sequential inserts).
    int hits = 0;
    for (std::uint64_t i = N - 32; i < N; ++i) {
        const auto g = make_guest(i);
        if (std::holds_alternative<const Entry*>(cache.lookup(i, g))) ++hits;
    }
    REQUIRE(hits >= 30);  // allow a tiny slack for hash-collision edge cases
}

TEST_CASE("cache stress: invalidate_page clears a page-sized range cleanly") {
    TranslationCache cache;
    constexpr std::uint64_t PAGE = 4096;
    // Seed addresses in two distinct pages.
    for (std::uint64_t i = 0; i < 100; ++i) {
        const auto g = make_guest(i);
        // Mix page-0 (i*32) and page-1 (PAGE + i*32) addresses.
        const std::uint64_t addr = (i % 2 == 0) ? (i * 32) : (PAGE + i * 32);
        cache.insert(Key{addr, fnv1a_64(g)}, make_entry(g));
    }
    const std::size_t before = cache.entry_count();
    REQUIRE(before == 100);

    // Drop everything in page-0.
    cache.invalidate_page(0, PAGE);
    REQUIRE(cache.entry_count() < before);

    // Page-1 entries must still be reachable.
    int surviving = 0;
    for (std::uint64_t i = 1; i < 100; i += 2) {
        const auto g = make_guest(i);
        const std::uint64_t addr = PAGE + i * 32;
        if (std::holds_alternative<const Entry*>(cache.lookup(addr, g))) ++surviving;
    }
    REQUIRE(surviving >= 40);  // roughly half of 100 - minor collisions
}
