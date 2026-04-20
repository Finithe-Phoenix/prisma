// core/tests/test_translation_cache.cpp — unit tests for the in-memory
// translation cache (Pillar 4 seed).

#include <catch2/catch_test_macros.hpp>
#include <cstdint>
#include <filesystem>
#include <fstream>
#include <variant>
#include <vector>

#include "prisma/translation_cache.hpp"

using namespace prisma::cache;

namespace {

// Small helper to make an Entry without ceremony.
Entry make_entry(std::vector<std::uint8_t> code,
                 std::size_t guest_size,
                 std::uint64_t content_hash) {
    return Entry{std::move(code), guest_size, content_hash};
}

}  // namespace

TEST_CASE("fnv1a_64 hashes deterministically and differently") {
    const std::vector<std::uint8_t> a{1, 2, 3, 4};
    const std::vector<std::uint8_t> b{1, 2, 3, 5};
    REQUIRE(fnv1a_64(a) == fnv1a_64(a));
    REQUIRE(fnv1a_64(a) != fnv1a_64(b));

    const std::vector<std::uint8_t> empty;
    REQUIRE(fnv1a_64(empty) == 0xcbf29ce484222325ULL);  // FNV offset basis
}

TEST_CASE("TranslationCache: insert then lookup hits") {
    TranslationCache cache;
    const std::vector<std::uint8_t> guest{0x48, 0xC3};   // REX + RET
    const std::uint64_t addr = 0x1000;
    const std::uint64_t hash = fnv1a_64(guest);

    REQUIRE(cache.insert(Key{addr, hash},
                         make_entry({0x1F, 0x20, 0x03, 0xD5}, guest.size(), hash)));

    auto res = cache.lookup(addr, guest);
    REQUIRE(std::holds_alternative<const Entry*>(res));
    const Entry* e = std::get<const Entry*>(res);
    REQUIRE(e != nullptr);
    REQUIRE(e->guest_size == guest.size());
    REQUIRE(e->code_bytes.size() == 4);
}

TEST_CASE("TranslationCache: lookup miss on unknown address") {
    TranslationCache cache;
    const std::vector<std::uint8_t> guest{0xC3};
    auto res = cache.lookup(0xDEADBEEFULL, guest);
    REQUIRE(std::holds_alternative<MissReason>(res));
    REQUIRE(std::get<MissReason>(res) == MissReason::UnknownAddress);
}

TEST_CASE("TranslationCache: SMC detection via content hash mismatch") {
    TranslationCache cache;
    const std::uint64_t addr = 0x2000;
    const std::vector<std::uint8_t> original{0x48, 0xC3};        // original code
    const std::uint64_t h0 = fnv1a_64(original);

    cache.insert(Key{addr, h0},
                 make_entry({0x01, 0x02, 0x03, 0x04}, original.size(), h0));

    // Simulate self-modifying code: same address, different bytes.
    const std::vector<std::uint8_t> modified{0x48, 0x90, 0xC3};

    auto res = cache.lookup(addr, modified);
    REQUIRE(std::holds_alternative<MissReason>(res));
    REQUIRE(std::get<MissReason>(res) == MissReason::StaleContent);
}

TEST_CASE("TranslationCache: upsert replaces and preserves addr→hash index") {
    TranslationCache cache;
    const std::uint64_t addr = 0x3000;
    const std::vector<std::uint8_t> v1{0xC3};
    const std::vector<std::uint8_t> v2{0x90, 0xC3};
    const std::uint64_t h1 = fnv1a_64(v1);
    const std::uint64_t h2 = fnv1a_64(v2);

    cache.insert(Key{addr, h1}, make_entry({0x11}, v1.size(), h1));
    REQUIRE(cache.entry_count() == 1);

    cache.upsert(Key{addr, h2}, make_entry({0x22}, v2.size(), h2));
    REQUIRE(cache.entry_count() == 1);  // old one evicted

    // Looking up v1 now reports stale.
    auto r1 = cache.lookup(addr, v1);
    REQUIRE(std::get<MissReason>(r1) == MissReason::StaleContent);

    // Looking up v2 hits and the returned bytes reflect the new entry.
    auto r2 = cache.lookup(addr, v2);
    const Entry* e2 = std::get<const Entry*>(r2);
    REQUIRE(e2->code_bytes == std::vector<std::uint8_t>{0x22});
}

TEST_CASE("TranslationCache: invalidate_page drops entries in range") {
    TranslationCache cache;
    // Four entries across two 4 KiB pages.
    const std::vector<std::uint8_t> b{0xC3};
    const std::uint64_t h = fnv1a_64(b);

    cache.insert(Key{0x1000, h}, make_entry({0x01}, 1, h));
    cache.insert(Key{0x1500, h}, make_entry({0x02}, 1, h));
    cache.insert(Key{0x2000, h}, make_entry({0x03}, 1, h));
    cache.insert(Key{0x2800, h}, make_entry({0x04}, 1, h));
    REQUIRE(cache.entry_count() == 4);

    cache.invalidate_page(0x1000, 4096);

    REQUIRE(cache.entry_count() == 2);
    REQUIRE(std::holds_alternative<MissReason>(cache.lookup(0x1000, b)));
    REQUIRE(std::holds_alternative<MissReason>(cache.lookup(0x1500, b)));
    REQUIRE(std::holds_alternative<const Entry*>(cache.lookup(0x2000, b)));
    REQUIRE(std::holds_alternative<const Entry*>(cache.lookup(0x2800, b)));
}

TEST_CASE("TranslationCache: same bytes at different addresses are independent") {
    TranslationCache cache;
    const std::vector<std::uint8_t> bytes{0x48, 0xC3};
    const std::uint64_t h = fnv1a_64(bytes);

    cache.insert(Key{0x1000, h}, make_entry({0xAA}, 2, h));
    cache.insert(Key{0x2000, h}, make_entry({0xBB}, 2, h));
    REQUIRE(cache.entry_count() == 2);

    const Entry* a = std::get<const Entry*>(cache.lookup(0x1000, bytes));
    const Entry* b = std::get<const Entry*>(cache.lookup(0x2000, bytes));
    REQUIRE(a->code_bytes[0] == 0xAA);
    REQUIRE(b->code_bytes[0] == 0xBB);
}

TEST_CASE("TranslationCache: insert is idempotent on exact-key duplicate") {
    TranslationCache cache;
    const std::vector<std::uint8_t> bytes{0xC3};
    const std::uint64_t h = fnv1a_64(bytes);
    REQUIRE(cache.insert(Key{0x1000, h}, make_entry({0x01}, 1, h)));
    REQUIRE_FALSE(cache.insert(Key{0x1000, h}, make_entry({0x02}, 1, h)));
    // First entry still present.
    const Entry* e = std::get<const Entry*>(cache.lookup(0x1000, bytes));
    REQUIRE(e->code_bytes[0] == 0x01);
}

// ---------------------------------------------------------------------------
// F1-CA-005: LRU eviction.
// ---------------------------------------------------------------------------

TEST_CASE("TranslationCache: set_max_entries(0) keeps cache unbounded") {
    TranslationCache cache;
    const std::vector<std::uint8_t> b{0xC3};
    const std::uint64_t h = fnv1a_64(b);

    // Default is 0 = unlimited; insert many entries.
    for (std::uint64_t i = 0; i < 50; ++i) {
        cache.insert(Key{0x10000 + i, h}, make_entry({static_cast<std::uint8_t>(i)}, 1, h));
    }
    REQUIRE(cache.entry_count() == 50);
}

TEST_CASE("TranslationCache: bounded cache evicts the least-recently-used entry") {
    TranslationCache cache;
    cache.set_max_entries(3);
    REQUIRE(cache.max_entries() == 3);

    const std::vector<std::uint8_t> b{0xC3};
    const std::uint64_t h = fnv1a_64(b);

    cache.insert(Key{0x1000, h}, make_entry({0x01}, 1, h));
    cache.insert(Key{0x2000, h}, make_entry({0x02}, 1, h));
    cache.insert(Key{0x3000, h}, make_entry({0x03}, 1, h));
    REQUIRE(cache.entry_count() == 3);

    // Touch 0x1000 so it becomes MRU. The LRU is now 0x2000.
    (void)cache.lookup(0x1000, b);

    // Insert a fourth → must evict 0x2000.
    cache.insert(Key{0x4000, h}, make_entry({0x04}, 1, h));
    REQUIRE(cache.entry_count() == 3);

    REQUIRE(std::holds_alternative<const Entry*>(cache.lookup(0x1000, b)));
    REQUIRE(std::holds_alternative<MissReason>(cache.lookup(0x2000, b)));
    REQUIRE(std::get<MissReason>(cache.lookup(0x2000, b)) == MissReason::UnknownAddress);
    REQUIRE(std::holds_alternative<const Entry*>(cache.lookup(0x3000, b)));
    REQUIRE(std::holds_alternative<const Entry*>(cache.lookup(0x4000, b)));
}

TEST_CASE("TranslationCache: lookup hit bumps LRU age") {
    TranslationCache cache;
    cache.set_max_entries(2);
    const std::vector<std::uint8_t> b{0xC3};
    const std::uint64_t h = fnv1a_64(b);

    cache.insert(Key{0x1000, h}, make_entry({0x01}, 1, h));
    cache.insert(Key{0x2000, h}, make_entry({0x02}, 1, h));
    // Access 0x1000 repeatedly; it should survive evictions over 0x2000.
    for (int i = 0; i < 5; ++i) (void)cache.lookup(0x1000, b);

    cache.insert(Key{0x3000, h}, make_entry({0x03}, 1, h));
    REQUIRE(std::holds_alternative<const Entry*>(cache.lookup(0x1000, b)));
    REQUIRE(std::holds_alternative<MissReason>(cache.lookup(0x2000, b)));
}

// ---------------------------------------------------------------------------
// F1-CA-003: persistent on-disk format round-trip.
// ---------------------------------------------------------------------------

TEST_CASE("TranslationCache: save / load round-trips a populated cache") {
    // Populate a reference cache.
    TranslationCache original;
    const std::vector<std::uint8_t> guest_a{0x48, 0xC3};
    const std::vector<std::uint8_t> guest_b{0xC3};
    const std::uint64_t ha = fnv1a_64(guest_a);
    const std::uint64_t hb = fnv1a_64(guest_b);

    original.insert(Key{0x1000, ha},
                    make_entry({0xAA, 0xBB, 0xCC, 0xDD}, guest_a.size(), ha));
    original.insert(Key{0x2000, hb},
                    make_entry({0x11, 0x22}, guest_b.size(), hb));

    // Write to a temp file.
    const auto tmp = std::filesystem::temp_directory_path()
                   / "prisma_cache_roundtrip.bin";
    {
        auto err = original.save_to_file(tmp);
        REQUIRE_FALSE(err.has_value());
    }

    // Load into a fresh cache; must reproduce original entries.
    TranslationCache loaded;
    {
        auto err = loaded.load_from_file(tmp);
        REQUIRE_FALSE(err.has_value());
    }
    REQUIRE(loaded.entry_count() == 2);

    auto r1 = loaded.lookup(0x1000, guest_a);
    REQUIRE(std::holds_alternative<const Entry*>(r1));
    const Entry* e1 = std::get<const Entry*>(r1);
    REQUIRE(e1->code_bytes == std::vector<std::uint8_t>{0xAA, 0xBB, 0xCC, 0xDD});
    REQUIRE(e1->guest_size == guest_a.size());

    auto r2 = loaded.lookup(0x2000, guest_b);
    REQUIRE(std::holds_alternative<const Entry*>(r2));
    const Entry* e2 = std::get<const Entry*>(r2);
    REQUIRE(e2->code_bytes == std::vector<std::uint8_t>{0x11, 0x22});

    std::filesystem::remove(tmp);
}

TEST_CASE("TranslationCache: save on empty cache writes only the header") {
    TranslationCache empty_cache;
    const auto tmp = std::filesystem::temp_directory_path()
                   / "prisma_cache_empty.bin";
    auto err = empty_cache.save_to_file(tmp);
    REQUIRE_FALSE(err.has_value());

    // Exactly 32 bytes of header, no entries.
    REQUIRE(std::filesystem::file_size(tmp) == 32u);

    TranslationCache loaded;
    REQUIRE_FALSE(loaded.load_from_file(tmp).has_value());
    REQUIRE(loaded.entry_count() == 0);

    std::filesystem::remove(tmp);
}

TEST_CASE("TranslationCache: load on missing file returns OpenFailed") {
    TranslationCache cache;
    auto err = cache.load_from_file("/tmp/definitely_does_not_exist_prisma_cache.xyz");
    REQUIRE(err.has_value());
    REQUIRE(*err == TranslationCache::IoError::OpenFailed);
}

TEST_CASE("TranslationCache: load rejects a file with wrong magic") {
    const auto tmp = std::filesystem::temp_directory_path()
                   / "prisma_cache_badmagic.bin";
    {
        std::ofstream os(tmp, std::ios::binary | std::ios::trunc);
        // Write 64 bytes of garbage with the wrong magic.
        const char bad[8] = {'N', 'O', 'P', 'E', '\0', '\0', '\0', '\0'};
        os.write(bad, 8);
        char padding[56]{};
        os.write(padding, 56);
    }

    TranslationCache cache;
    auto err = cache.load_from_file(tmp);
    REQUIRE(err.has_value());
    REQUIRE(*err == TranslationCache::IoError::BadMagic);
    std::filesystem::remove(tmp);
}

TEST_CASE("TranslationCache: load leaves cache unchanged on error") {
    // Populate, save valid, then corrupt and load again; the cache should
    // retain its original state.
    TranslationCache cache;
    const std::vector<std::uint8_t> g{0xC3};
    const std::uint64_t h = fnv1a_64(g);
    cache.insert(Key{0x1000, h}, make_entry({0x42}, 1, h));
    REQUIRE(cache.entry_count() == 1);

    // Attempt to load a missing file.
    auto err = cache.load_from_file("/tmp/no_such_prisma_cache.bin");
    REQUIRE(err.has_value());
    REQUIRE(cache.entry_count() == 1);  // still 1 — untouched.
}

// ---------------------------------------------------------------------------
// Per-entry stats (F1-CA-007)
// ---------------------------------------------------------------------------

TEST_CASE("TranslationCache: stats_for reports 0 hits on a fresh insert") {
    TranslationCache cache;
    const std::vector<std::uint8_t> g{0x90};
    const std::uint64_t h = fnv1a_64(g);
    const Key key{0x1000, h};
    cache.insert(key, make_entry({0xAA}, 1, h));

    auto s = cache.stats_for(key);
    REQUIRE(s.has_value());
    REQUIRE(s->hit_count == 0);
    REQUIRE(s->last_used_tick > 0);
}

TEST_CASE("TranslationCache: lookup hits bump hit_count") {
    TranslationCache cache;
    const std::vector<std::uint8_t> g{0x90};
    const std::uint64_t h = fnv1a_64(g);
    const Key key{0x1000, h};
    cache.insert(key, make_entry({0xAA}, 1, h));

    (void)cache.lookup(0x1000, g);
    (void)cache.lookup(0x1000, g);
    (void)cache.lookup(0x1000, g);

    auto s = cache.stats_for(key);
    REQUIRE(s.has_value());
    REQUIRE(s->hit_count == 3);
}

TEST_CASE("TranslationCache: stale-content miss does NOT bump hit_count") {
    TranslationCache cache;
    const std::vector<std::uint8_t> g{0x90};
    const std::uint64_t h = fnv1a_64(g);
    const Key key{0x1000, h};
    cache.insert(key, make_entry({0xAA}, 1, h));

    // Lookup with modified content — should miss with StaleContent.
    const std::vector<std::uint8_t> modified{0x91};
    auto r = cache.lookup(0x1000, modified);
    REQUIRE(std::holds_alternative<MissReason>(r));

    auto s = cache.stats_for(key);
    REQUIRE(s.has_value());
    REQUIRE(s->hit_count == 0);
}

TEST_CASE("TranslationCache: reset_hit_counts zeroes the tally") {
    TranslationCache cache;
    const std::vector<std::uint8_t> g{0x90};
    const std::uint64_t h = fnv1a_64(g);
    const Key key{0x1000, h};
    cache.insert(key, make_entry({0xAA}, 1, h));
    (void)cache.lookup(0x1000, g);
    (void)cache.lookup(0x1000, g);

    cache.reset_hit_counts();
    auto s = cache.stats_for(key);
    REQUIRE(s.has_value());
    REQUIRE(s->hit_count == 0);
}

TEST_CASE("TranslationCache: stats_for returns nullopt for unknown key") {
    TranslationCache cache;
    auto s = cache.stats_for(Key{0xDEAD, 0xBEEF});
    REQUIRE_FALSE(s.has_value());
}

TEST_CASE("TranslationCache: upsert resets hit_count to 0 (fresh entry)") {
    TranslationCache cache;
    const std::vector<std::uint8_t> g1{0x90};
    const std::uint64_t h1 = fnv1a_64(g1);
    const Key key1{0x1000, h1};
    cache.insert(key1, make_entry({0xAA}, 1, h1));
    (void)cache.lookup(0x1000, g1);
    (void)cache.lookup(0x1000, g1);

    // Upsert with the SAME hash — semantically overwrites the entry.
    cache.upsert(key1, make_entry({0xBB}, 1, h1));
    auto s = cache.stats_for(key1);
    REQUIRE(s.has_value());
    REQUIRE(s->hit_count == 0);
}
