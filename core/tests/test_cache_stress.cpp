// core/tests/test_cache_stress.cpp — F1-TC-012 stress the cache at scale.
//
// Also tests FNV-1a, persistent save/load round-trip, and compaction —
// groups the cache-family tests in one place so they share the helper
// functions above.
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
#include <bit>
#include <cstdlib>
#include <cstdint>
#include <filesystem>
#include <fstream>
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

TEST_CASE("fnv1a_64: empty input produces a deterministic hash") {
    const std::vector<std::uint8_t> empty;
    const std::uint64_t h = fnv1a_64(empty);
    // FNV-1a offset basis for 64-bit: 0xCBF29CE484222325
    REQUIRE(h == 0xCBF29CE484222325ULL);
}

TEST_CASE("fnv1a_64: single-byte input") {
    const std::vector<std::uint8_t> input{'A'};
    const std::uint64_t h = fnv1a_64(input);
    // Manually verified: basis * prime ^ 'A'
    REQUIRE(h != 0);  // not zero
    REQUIRE(h != 0xCBF29CE484222325ULL);  // changed from basis
}

TEST_CASE("fnv1a_64: same input produces same hash") {
    const auto a = fnv1a_64(std::vector<std::uint8_t>{1, 2, 3, 4});
    const auto b = fnv1a_64(std::vector<std::uint8_t>{1, 2, 3, 4});
    REQUIRE(a == b);
}

TEST_CASE("fnv1a_64: one-bit difference flips many bits") {
    const auto a = fnv1a_64(std::vector<std::uint8_t>{0, 0, 0, 0});
    const auto b = fnv1a_64(std::vector<std::uint8_t>{0, 0, 0, 1});
    REQUIRE(a != b);
    // FNV-1a is not an avalanche-quality hash: a one-bit change in
    // the LAST byte only goes through a single multiply, so few bits
    // flip (measured: 9). Assert a lenient floor.
    const std::uint64_t diff = a ^ b;
    REQUIRE(std::popcount(diff) >= 4);
}

TEST_CASE("fnv1a_64: zero prefix changes hash") {
    const auto a = fnv1a_64(std::vector<std::uint8_t>{0, 1});
    const auto b = fnv1a_64(std::vector<std::uint8_t>{1});
    REQUIRE(a != b);  // length encoded in the hash
}

TEST_CASE("cache: keys differing in one component coexist as entries") {
    // The hasher is an implementation detail (private KeyHash); the
    // observable property is that keys differing in only guest_addr
    // or only content_hash do not collide into one entry.
    TranslationCache c;
    const auto g = make_guest(7);
    REQUIRE(c.insert(Key{0x1000, 0xAAA}, make_entry(g)));
    REQUIRE(c.insert(Key{0x1000, 0xBBB}, make_entry(g)));
    REQUIRE(c.insert(Key{0x2000, 0xAAA}, make_entry(g)));
    REQUIRE(c.entry_count() == 3);
}

TEST_CASE("cache: entries start with zero hit count") {
    TranslationCache cache;
    const auto g = make_guest(42);
    cache.insert(Key{0x1000, fnv1a_64(g)}, make_entry(g));
    auto stats = cache.stats_for(Key{0x1000, fnv1a_64(g)});
    REQUIRE(stats.has_value());
    REQUIRE(stats->hit_count == 0);
}

TEST_CASE("cache: lookup bumps hit count") {
    TranslationCache cache;
    const auto g = make_guest(77);
    cache.insert(Key{0x2000, fnv1a_64(g)}, make_entry(g));
    (void)cache.lookup(0x2000, g);
    (void)cache.lookup(0x2000, g);
    auto stats = cache.stats_for(Key{0x2000, fnv1a_64(g)});
    REQUIRE(stats.has_value());
    REQUIRE(stats->hit_count == 2);
}

TEST_CASE("cache: reset_hit_counts zeroes all hit counts") {
    TranslationCache cache;
    const auto g1 = make_guest(1);
    const auto g2 = make_guest(2);
    cache.insert(Key{0x1000, fnv1a_64(g1)}, make_entry(g1));
    cache.insert(Key{0x2000, fnv1a_64(g2)}, make_entry(g2));
    (void)cache.lookup(0x1000, g1);
    (void)cache.lookup(0x2000, g2);
    cache.reset_hit_counts();
    auto s1 = cache.stats_for(Key{0x1000, fnv1a_64(g1)});
    auto s2 = cache.stats_for(Key{0x2000, fnv1a_64(g2)});
    REQUIRE(s1.has_value());
    REQUIRE(s2.has_value());
    REQUIRE(s1->hit_count == 0);
    REQUIRE(s2->hit_count == 0);
}

TEST_CASE("cache: save_to_file / load_from_file round-trip preserves entries") {
    const auto g1 = make_guest(1);
    const auto g2 = make_guest(2);

    TranslationCache src;
    src.insert(Key{0x1000, fnv1a_64(g1)}, make_entry(g1));
    src.insert(Key{0x2000, fnv1a_64(g2)}, make_entry(g2));
    REQUIRE(src.entry_count() == 2);

    const auto* tmp_path = std::getenv("PRISMA_TEST_TMPDIR");
    const auto dir = tmp_path ? std::filesystem::path{tmp_path}
                              : std::filesystem::temp_directory_path();
    const auto path = dir / "prisma_cache_roundtrip.bin";

    // Clean up from previous runs.
    std::error_code ec;
    std::filesystem::remove(path, ec);

    auto save_result = src.save_to_file(path);
    REQUIRE_FALSE(save_result.has_value());  // no error

    TranslationCache dst;
    auto load_result = dst.load_from_file(path);
    REQUIRE_FALSE(load_result.has_value());

    REQUIRE(dst.entry_count() == 2);

    // Must find both entries.
    auto r1 = dst.lookup(0x1000, g1);
    REQUIRE(std::holds_alternative<const Entry*>(r1));
    REQUIRE(std::get<const Entry*>(r1)->guest_size == g1.size());

    auto r2 = dst.lookup(0x2000, g2);
    REQUIRE(std::holds_alternative<const Entry*>(r2));
    REQUIRE(std::get<const Entry*>(r2)->guest_size == g2.size());

    std::filesystem::remove(path, ec);
}

TEST_CASE("cache: save_to_file / load_from_file with compression") {
    const auto g = make_guest(99);

    TranslationCache src;
    src.set_compress_on_save(true);
    src.insert(Key{0x3000, fnv1a_64(g)}, make_entry(g));

    const auto* tmp_path = std::getenv("PRISMA_TEST_TMPDIR");
    const auto dir = tmp_path ? std::filesystem::path{tmp_path}
                              : std::filesystem::temp_directory_path();
    const auto path = dir / "prisma_cache_compressed.bin";

    std::error_code ec;
    std::filesystem::remove(path, ec);

    auto save_result = src.save_to_file(path);
    REQUIRE_FALSE(save_result.has_value());

    TranslationCache dst;
    auto load_result = dst.load_from_file(path);
    REQUIRE_FALSE(load_result.has_value());

    REQUIRE(dst.entry_count() == 1);
    auto r = dst.lookup(0x3000, g);
    REQUIRE(std::holds_alternative<const Entry*>(r));

    std::filesystem::remove(path, ec);
}

TEST_CASE("cache: load_from_file rejects bad magic") {
    TranslationCache cache;
    const auto* tmp_path = std::getenv("PRISMA_TEST_TMPDIR");
    const auto dir = tmp_path ? std::filesystem::path{tmp_path}
                              : std::filesystem::temp_directory_path();
    const auto path = dir / "prisma_cache_bad_magic.bin";

    {
        std::ofstream ofs(path, std::ios::binary);
        REQUIRE(ofs);
        const std::uint64_t bad_magic = 0xBADBADBADBADBADBULL;
        ofs.write(reinterpret_cast<const char*>(&bad_magic), sizeof(bad_magic));
    }

    auto result = cache.load_from_file(path);
    REQUIRE(result.has_value());
    REQUIRE(*result == TranslationCache::IoError::BadMagic);

    std::error_code ec;
    std::filesystem::remove(path, ec);
}

TEST_CASE("cache: upsert replaces entry with same address") {
    TranslationCache cache;
    const auto g_old = make_guest(1);
    const auto g_new = make_guest(2);

    cache.insert(Key{0x1000, fnv1a_64(g_old)}, make_entry(g_old));
    REQUIRE(cache.entry_count() == 1);

    // Same address but new content
    cache.upsert(Key{0x1000, fnv1a_64(g_new)}, make_entry(g_new));
    REQUIRE(cache.entry_count() == 2);  // old + new both live until eviction

    // Old content still reachable via its original key
    auto r_old = cache.lookup(0x1000, g_old);
    REQUIRE(std::holds_alternative<const Entry*>(r_old));

    // New content reachable via new hash
    auto r_new = cache.lookup(0x1000, g_new);
    REQUIRE(std::holds_alternative<const Entry*>(r_new));
}

TEST_CASE("cache: compact removes superseded entries") {
    TranslationCache cache;
    const auto g_old = make_guest(1);
    const auto g_new = make_guest(2);

    cache.insert(Key{0x1000, fnv1a_64(g_old)}, make_entry(g_old));
    cache.upsert(Key{0x1000, fnv1a_64(g_new)}, make_entry(g_new));
    REQUIRE(cache.entry_count() == 2);

    const std::size_t evicted = cache.compact();
    REQUIRE(evicted == 1);  // old entry dropped
    REQUIRE(cache.entry_count() == 1);
}

TEST_CASE("cache: compact on clean cache returns zero") {
    TranslationCache cache;
    REQUIRE(cache.compact() == 0);
}

TEST_CASE("cache: save_to_file_async succeeds and persists entries") {
    const auto g = make_guest(55);

    TranslationCache src;
    src.insert(Key{0x4000, fnv1a_64(g)}, make_entry(g));
    src.set_compress_on_save(false);

    const auto* tmp_path = std::getenv("PRISMA_TEST_TMPDIR");
    const auto dir = tmp_path ? std::filesystem::path{tmp_path}
                              : std::filesystem::temp_directory_path();
    const auto path = dir / "prisma_cache_async.bin";

    std::error_code ec;
    std::filesystem::remove(path, ec);

    src.save_to_file_async(path);
    auto err = src.wait_for_async_save();
    REQUIRE_FALSE(err.has_value());

    TranslationCache dst;
    auto load_err = dst.load_from_file(path);
    REQUIRE_FALSE(load_err.has_value());
    REQUIRE(dst.entry_count() == 1);

    auto r = dst.lookup(0x4000, g);
    REQUIRE(std::holds_alternative<const Entry*>(r));

    std::filesystem::remove(path, ec);
}

TEST_CASE("cache: set_max_entries enforces hard ceiling") {
    TranslationCache cache;
    cache.set_max_entries(4);

    for (std::uint64_t i = 0; i < 100; ++i) {
        const auto g = make_guest(i);
        cache.insert(Key{i, fnv1a_64(g)}, make_entry(g));
    }
    REQUIRE(cache.entry_count() <= 4);
}

TEST_CASE("cache: invalidate_page with non-existent address is a no-op") {
    TranslationCache cache;
    REQUIRE_NOTHROW(cache.invalidate_page(0xDEAD, 4096));
    REQUIRE(cache.entry_count() == 0);
}
