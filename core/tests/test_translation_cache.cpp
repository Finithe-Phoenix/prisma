// core/tests/test_translation_cache.cpp — unit tests for the in-memory
// translation cache (Pillar 4 seed).

#include <catch2/catch_test_macros.hpp>
#include <cstdint>
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
