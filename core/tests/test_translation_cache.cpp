// core/tests/test_translation_cache.cpp — unit tests for the in-memory
// translation cache (Pillar 4 seed).

#include <catch2/catch_test_macros.hpp>
#include <cstdint>
#include <filesystem>
#include <fstream>
#include <variant>
#include <vector>

#include "prisma/compress.hpp"
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

// ---------------------------------------------------------------------------
// Byte-budget eviction (F1-CA-006)
// ---------------------------------------------------------------------------

TEST_CASE("TranslationCache: total_code_bytes sums entry code sizes") {
    TranslationCache cache;
    const std::vector<std::uint8_t> g{0x01};
    const std::uint64_t h = fnv1a_64(g);
    cache.insert(Key{0x1, h}, make_entry({0xAA, 0xBB, 0xCC}, 1, h));
    cache.insert(Key{0x2, h}, make_entry({0xDD, 0xEE}, 1, h));
    REQUIRE(cache.total_code_bytes() == 5u);
}

TEST_CASE("TranslationCache: max_bytes bound evicts LRU entries") {
    TranslationCache cache;
    cache.set_max_bytes(10);

    const std::vector<std::uint8_t> g1{0x01};
    const std::uint64_t h1 = fnv1a_64(g1);
    const std::vector<std::uint8_t> g2{0x02};
    const std::uint64_t h2 = fnv1a_64(g2);
    const std::vector<std::uint8_t> g3{0x03};
    const std::uint64_t h3 = fnv1a_64(g3);

    cache.insert(Key{0x1, h1}, make_entry(std::vector<std::uint8_t>(6, 0xAA), 1, h1));
    cache.insert(Key{0x2, h2}, make_entry(std::vector<std::uint8_t>(3, 0xBB), 1, h2));
    REQUIRE(cache.total_code_bytes() == 9u);
    REQUIRE(cache.entry_count() == 2);

    // Inserting 5 more bytes would push to 14; LRU (entry 1) is evicted.
    cache.insert(Key{0x3, h3}, make_entry(std::vector<std::uint8_t>(5, 0xCC), 1, h3));
    REQUIRE(cache.total_code_bytes() <= 10u);
    // Entry 1 (the LRU when we inserted 3) is gone; entries 2 and 3 survive.
    auto r = cache.lookup(0x1, g1);
    REQUIRE(std::holds_alternative<MissReason>(r));
}

TEST_CASE("TranslationCache: max_bytes and max_entries combine") {
    // Set both caps: entries <= 2, bytes <= 100. Either triggers.
    TranslationCache cache;
    cache.set_max_entries(2);
    cache.set_max_bytes(100);

    const std::vector<std::uint8_t> g1{0x01};
    const std::uint64_t h1 = fnv1a_64(g1);
    const std::vector<std::uint8_t> g2{0x02};
    const std::uint64_t h2 = fnv1a_64(g2);
    const std::vector<std::uint8_t> g3{0x03};
    const std::uint64_t h3 = fnv1a_64(g3);

    cache.insert(Key{0x1, h1}, make_entry({0xAA}, 1, h1));
    cache.insert(Key{0x2, h2}, make_entry({0xBB}, 1, h2));
    cache.insert(Key{0x3, h3}, make_entry({0xCC}, 1, h3));
    // max_entries trips first (3 > 2), LRU (key1) is evicted.
    REQUIRE(cache.entry_count() == 2);
    auto r = cache.lookup(0x1, g1);
    REQUIRE(std::holds_alternative<MissReason>(r));
}

TEST_CASE("TranslationCache: 0 max_bytes = unlimited") {
    TranslationCache cache;
    REQUIRE(cache.max_bytes() == 0);
    // Inserting lots of entries stays fine.
    for (std::uint64_t i = 0; i < 20; ++i) {
        const std::vector<std::uint8_t> g{static_cast<std::uint8_t>(i)};
        const std::uint64_t h = fnv1a_64(g);
        cache.insert(Key{i, h}, make_entry(std::vector<std::uint8_t>(8, 0xFF), 1, h));
    }
    REQUIRE(cache.entry_count() == 20);
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

// ---------------------------------------------------------------------------
// F1-CA-009 async save
// ---------------------------------------------------------------------------

TEST_CASE("TranslationCache: async save produces an on-disk file the sync loader can read") {
    TranslationCache src;
    const std::vector<std::uint8_t> g1{0x01, 0x02};
    const std::vector<std::uint8_t> g2{0x03, 0x04};
    const std::uint64_t h1 = fnv1a_64(g1);
    const std::uint64_t h2 = fnv1a_64(g2);
    src.insert(Key{0x1000, h1}, make_entry({0xAA, 0xBB}, g1.size(), h1));
    src.insert(Key{0x2000, h2}, make_entry({0xCC}, g2.size(), h2));

    const auto tmp = std::filesystem::temp_directory_path()
                   / "prisma_async_save.bin";
    src.save_to_file_async(tmp);
    auto err = src.wait_for_async_save();
    REQUIRE_FALSE(err.has_value());

    // Load into a fresh cache and verify both entries survive.
    TranslationCache dst;
    auto load_err = dst.load_from_file(tmp);
    REQUIRE_FALSE(load_err.has_value());
    REQUIRE(dst.entry_count() == 2);
    auto r = dst.lookup(0x1000, g1);
    REQUIRE(std::holds_alternative<const Entry*>(r));

    std::filesystem::remove(tmp);
}

TEST_CASE("TranslationCache: async save snapshot is isolated from post-call mutations") {
    TranslationCache cache;
    const std::vector<std::uint8_t> g1{0x01};
    const std::uint64_t h1 = fnv1a_64(g1);
    cache.insert(Key{0x1000, h1}, make_entry({0xAA}, 1, h1));

    const auto tmp = std::filesystem::temp_directory_path()
                   / "prisma_async_snap.bin";
    cache.save_to_file_async(tmp);

    // Insert an entry AFTER the async call but (ideally) before the
    // worker finishes writing. The snapshot captured before we changed
    // the cache should NOT include this one.
    const std::vector<std::uint8_t> g2{0x02};
    const std::uint64_t h2 = fnv1a_64(g2);
    cache.insert(Key{0x2000, h2}, make_entry({0xBB}, 1, h2));

    auto err = cache.wait_for_async_save();
    REQUIRE_FALSE(err.has_value());

    TranslationCache loaded;
    auto load_err = loaded.load_from_file(tmp);
    REQUIRE_FALSE(load_err.has_value());
    // Only the first entry was in the snapshot.
    REQUIRE(loaded.entry_count() == 1);

    std::filesystem::remove(tmp);
}

TEST_CASE("TranslationCache: wait_for_async_save is idempotent when no save pending") {
    TranslationCache cache;
    auto err = cache.wait_for_async_save();
    REQUIRE_FALSE(err.has_value());
    // Call twice — still fine.
    err = cache.wait_for_async_save();
    REQUIRE_FALSE(err.has_value());
}

TEST_CASE("TranslationCache: destructor joins an in-flight async save") {
    const auto tmp = std::filesystem::temp_directory_path()
                   / "prisma_async_dtor.bin";
    {
        TranslationCache cache;
        const std::vector<std::uint8_t> g{0x42};
        const std::uint64_t h = fnv1a_64(g);
        cache.insert(Key{0x1000, h}, make_entry({0x11}, 1, h));
        cache.save_to_file_async(tmp);
        // Deliberately DON'T wait — the destructor should join.
    }
    // File exists and is loadable.
    TranslationCache loaded;
    auto err = loaded.load_from_file(tmp);
    REQUIRE_FALSE(err.has_value());
    REQUIRE(loaded.entry_count() == 1);
    std::filesystem::remove(tmp);
}

TEST_CASE("TranslationCache: async save to an invalid path returns OpenFailed") {
    TranslationCache cache;
    const std::vector<std::uint8_t> g{0x01};
    const std::uint64_t h = fnv1a_64(g);
    cache.insert(Key{0x1000, h}, make_entry({0xAA}, 1, h));

    cache.save_to_file_async("/definitely/not/a/writable/path/prisma.bin");
    auto err = cache.wait_for_async_save();
    REQUIRE(err.has_value());
    REQUIRE(*err == TranslationCache::IoError::OpenFailed);
}

// ---------------------------------------------------------------------------
// F1-CA-010 zstd compression
// ---------------------------------------------------------------------------

TEST_CASE("TranslationCache: compress_on_save round-trips entries correctly") {
    TranslationCache src;
    src.set_compress_on_save(true);
    REQUIRE(src.compress_on_save());

    // Highly-compressible code to make the on-disk payload shrink.
    const std::vector<std::uint8_t> g{0xDE, 0xAD};
    const std::uint64_t h = fnv1a_64(g);
    std::vector<std::uint8_t> big(1024, 0xAA);
    big[0] = 0xDE;
    big[1] = 0xAD;
    src.insert(Key{0x1000, h}, Entry{big, g.size(), h});

    const auto tmp = std::filesystem::temp_directory_path()
                   / "prisma_compressed.bin";
    REQUIRE_FALSE(src.save_to_file(tmp).has_value());

    TranslationCache dst;
    auto err = dst.load_from_file(tmp);
    REQUIRE_FALSE(err.has_value());
    REQUIRE(dst.entry_count() == 1);

    auto r = dst.lookup(0x1000, g);
    REQUIRE(std::holds_alternative<const Entry*>(r));
    const Entry* e = std::get<const Entry*>(r);
    REQUIRE(e->code_bytes.size() == big.size());
    REQUIRE(e->code_bytes == big);

    // Compressed file must be smaller than the raw payload for
    // this pathologically-compressible input.
    const auto file_size = std::filesystem::file_size(tmp);
    REQUIRE(file_size < big.size());

    std::filesystem::remove(tmp);
}

TEST_CASE("TranslationCache: compress_on_save=false emits v2 uncompressed") {
    TranslationCache src;
    REQUIRE_FALSE(src.compress_on_save());

    const std::vector<std::uint8_t> g{0x01};
    const std::uint64_t h = fnv1a_64(g);
    src.insert(Key{0x2000, h}, make_entry({0xAA, 0xBB, 0xCC, 0xDD}, 1, h));

    const auto tmp = std::filesystem::temp_directory_path()
                   / "prisma_uncompressed_v2.bin";
    REQUIRE_FALSE(src.save_to_file(tmp).has_value());

    TranslationCache dst;
    auto err = dst.load_from_file(tmp);
    REQUIRE_FALSE(err.has_value());
    REQUIRE(dst.entry_count() == 1);

    std::filesystem::remove(tmp);
}

TEST_CASE("TranslationCache: reader accepts hand-crafted v1 files (backward compat)") {
    // Build a minimal valid v1 file by hand: magic, version=1, reserved=0,
    // cpu_fp=0, entry_count=1; then one entry with 2 code bytes.
    const auto tmp = std::filesystem::temp_directory_path()
                   / "prisma_legacy_v1.bin";
    {
        std::ofstream os(tmp, std::ios::binary | std::ios::trunc);
        auto put_u64 = [&](std::uint64_t v) {
            for (int i = 0; i < 8; ++i) {
                char c = static_cast<char>((v >> (8 * i)) & 0xFF);
                os.write(&c, 1);
            }
        };
        auto put_u32 = [&](std::uint32_t v) {
            for (int i = 0; i < 4; ++i) {
                char c = static_cast<char>((v >> (8 * i)) & 0xFF);
                os.write(&c, 1);
            }
        };
        put_u64(TranslationCache::kFileMagic);
        put_u32(1u);         // version = 1
        put_u32(0u);         // reserved
        put_u64(0ull);       // cpu_fingerprint
        put_u64(1ull);       // entry_count
        // Entry 1:
        put_u64(0x3000ull);                          // guest_addr
        const std::vector<std::uint8_t> g{0xAB};
        const std::uint64_t h = fnv1a_64(g);
        put_u64(h);                                   // content_hash
        put_u64(g.size());                            // guest_size
        put_u64(2ull);                                // code_size (v1 layout)
        const char code[2] = {0x11, 0x22};
        os.write(code, 2);
    }

    TranslationCache cache;
    auto err = cache.load_from_file(tmp);
    REQUIRE_FALSE(err.has_value());
    REQUIRE(cache.entry_count() == 1);

    std::filesystem::remove(tmp);
}

TEST_CASE("TranslationCache: corrupt compressed payload is rejected") {
    // Build a v2 file whose "compressed" bytes are garbage. load should
    // fail without touching the destination cache.
    const auto tmp = std::filesystem::temp_directory_path()
                   / "prisma_corrupt_compressed.bin";
    {
        std::ofstream os(tmp, std::ios::binary | std::ios::trunc);
        auto put_u64 = [&](std::uint64_t v) {
            for (int i = 0; i < 8; ++i) {
                char c = static_cast<char>((v >> (8 * i)) & 0xFF);
                os.write(&c, 1);
            }
        };
        auto put_u32 = [&](std::uint32_t v) {
            for (int i = 0; i < 4; ++i) {
                char c = static_cast<char>((v >> (8 * i)) & 0xFF);
                os.write(&c, 1);
            }
        };
        put_u64(TranslationCache::kFileMagic);
        put_u32(TranslationCache::kFileVersion);     // v2
        put_u32(TranslationCache::kFlagCompressed);  // compressed bit set
        put_u64(0ull);
        put_u64(1ull);
        put_u64(0x4000ull);
        put_u64(0xCAFEull);
        put_u64(4ull);
        put_u64(4ull);                                // stored_size
        put_u64(4ull);                                // uncompressed_size
        const char junk[4] = {0, 0, 0, 0};            // NOT a zstd frame
        os.write(junk, 4);
    }
    TranslationCache cache;
    // Pre-populate so we can check it's not touched.
    const std::vector<std::uint8_t> g{0x01};
    const std::uint64_t h = fnv1a_64(g);
    cache.insert(Key{0x1, h}, make_entry({0xFF}, 1, h));
    REQUIRE(cache.entry_count() == 1);

    auto err = cache.load_from_file(tmp);
    REQUIRE(err.has_value());
    REQUIRE(cache.entry_count() == 1);  // untouched

    std::filesystem::remove(tmp);
}

TEST_CASE("zstd_compress + zstd_decompress round-trip") {
    std::vector<std::uint8_t> src(4096);
    for (std::size_t i = 0; i < src.size(); ++i) {
        src[i] = static_cast<std::uint8_t>(i * 31u);
    }
    auto compressed = zstd_compress(src);
    REQUIRE_FALSE(compressed.empty());
    auto round_trip = zstd_decompress(compressed);
    REQUIRE(round_trip.has_value());
    REQUIRE(*round_trip == src);
}

TEST_CASE("zstd_compress on empty input returns empty") {
    const std::vector<std::uint8_t> empty;
    auto out = zstd_compress(empty);
    REQUIRE(out.empty());
    auto back = zstd_decompress(out);
    REQUIRE(back.has_value());
    REQUIRE(back->empty());
}

// ---------------------------------------------------------------------
// F1-CA-008 compaction
// ---------------------------------------------------------------------

TEST_CASE("TranslationCache: compact() drops stale SMC entries") {
    TranslationCache c;
    const std::uint64_t addr = 0x4000;
    const std::vector<std::uint8_t> v0{0x90, 0x90};
    const std::vector<std::uint8_t> v1{0x91, 0x91};
    const std::vector<std::uint8_t> v2{0x92, 0x92};
    const auto h0 = fnv1a_64(v0);
    const auto h1 = fnv1a_64(v1);
    const auto h2 = fnv1a_64(v2);

    // Insert three SMC versions at the same guest_addr. We use
    // `insert` (not `upsert`) because upsert already cleans up the
    // prior hash for the same address; insert leaves stale entries
    // behind, which is exactly what compact() is supposed to mop up.
    // addr_to_hash_ tracks only the latest content_hash per addr.
    c.insert(Key{addr, h0}, make_entry({0xA0}, v0.size(), h0));
    c.insert(Key{addr, h1}, make_entry({0xA1}, v1.size(), h1));
    c.insert(Key{addr, h2}, make_entry({0xA2}, v2.size(), h2));

    REQUIRE(c.entry_count() == 3);
    const auto evicted = c.compact();
    REQUIRE(evicted == 2);
    REQUIRE(c.entry_count() == 1);

    // The surviving entry is the most recent one (h2).
    REQUIRE(c.stats_for(Key{addr, h2}).has_value());
    REQUIRE_FALSE(c.stats_for(Key{addr, h0}).has_value());
    REQUIRE_FALSE(c.stats_for(Key{addr, h1}).has_value());
}

TEST_CASE("TranslationCache: compact() is a no-op when nothing is stale") {
    TranslationCache c;
    c.insert(Key{0x1000, 0x1234}, make_entry({0x01}, 1, 0x1234));
    c.insert(Key{0x2000, 0x5678}, make_entry({0x02}, 1, 0x5678));
    c.insert(Key{0x3000, 0x9ABC}, make_entry({0x03}, 1, 0x9ABC));
    REQUIRE(c.compact() == 0);
    REQUIRE(c.entry_count() == 3);
}

TEST_CASE("TranslationCache: compact() on an empty cache returns 0") {
    TranslationCache c;
    REQUIRE(c.compact() == 0);
}
